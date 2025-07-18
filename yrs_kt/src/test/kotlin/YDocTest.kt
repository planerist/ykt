import com.planerist.ykt.*
import com.planerist.ykt.YDelta.YInsert
import org.junit.jupiter.api.assertThrows
import kotlin.test.Test
import kotlin.test.assertEquals

class YDocTest {
    @Test
    fun TestSnapshotState() {
        val d1 = YDoc(YDocOptions(1u, gc = false))

        val text = d1.getText("text")
        text.insert(0u, "hello")

        val prev = snapshot(d1)
        text.insert(5u, " world")

        val state = encodeStateFromSnapshotV2(d1, prev)

        val d2 = YDoc(YDocOptions(2u))
        val txt2 = d2.getText("text")
        applyUpdateV2(d2, state)

        assertEquals(txt2.toText(), "hello")
    }

    @Test
    fun TestStateAsUpdateDefault() {
        val d1 = YDoc(YDocOptions(1u, gc = false))

        val text = d1.getText("text")
        text.insert(0u, "hello")

        val state = encodeStateAsUpdateV2(d1)

        val d2 = YDoc(YDocOptions(2u))
        applyUpdateV2(d2, state)

        val txt2 = d2.getText("text")
        assertEquals(txt2.toText(), "hello")
    }

    @Test
    fun TestDelta() {
        val doc = YDoc()
        val text = doc.getText("text")
        text.insert(0u, "hello")
        val prev = snapshot(doc)
        text.insert(5u, " world")
        val next = snapshot(doc)

        val delta = text.toDelta(next, prev)
        assertEquals(
            listOf(
                YInsert(YValue.String("hello"), null),
                YInsert(YValue.String(" world"), null),
            ), delta
        )
    }

    @Test
    fun TestDeltaWithAttrs() {
        val doc = YDoc()
        val text = doc.getText("text")
        text.insert(0u, "hello")
        val prev = snapshot(doc)
        text.insert(5u, " world", "{\"bold\":true}")
        val next = snapshot(doc)

        val delta = text.toDelta(next, prev)
        assertEquals(
            listOf(
                YInsert(YValue.String("hello"), null),
                YInsert(YValue.String(" world"), mapOf("bold" to YValue.Bool(true))),
            ), delta
        )
    }

    @Test
    fun TestDeltaApplyDelta() {
        val doc = YDoc()
        val text = doc.getText("text")
        text.insert(0u, "hello")
        text.insert(5u, " world", "{\"bold\":true}")

        text.applyDelta(
            listOf(
                YDelta.YRetain(11u, null),
                YInsert(YValue.String("after"), null)
            )
        )

        assertEquals(
            listOf(
                YInsert(YValue.String("hello"), null),
                YInsert(YValue.String(" world"), mapOf("bold" to YValue.Bool(true))),
                YInsert(YValue.String("after"), null)
            ), text.toDelta()
        )
    }

    @Test
    fun TestDeltaApplyDelta2() {
        val doc = YDoc()
        val text = doc.getText("text")
        text.insert(0u, "hello")
        text.insert(5u, " world", "{\"bold\":true}")

        text.applyDelta(
            listOf(
                YDelta.YRetain(5u, null),
                YDelta.YRetain(2u, mapOf("italic" to YValue.Bool(true)))
            )
        )

        assertEquals(
            listOf(
                YInsert(YValue.String("hello"), null),
                YInsert(
                    YValue.String(" w"), mapOf(
                        "bold" to YValue.Bool(true),
                        "italic" to YValue.Bool(true)
                    )
                ),
                YInsert(YValue.String("orld"), mapOf("bold" to YValue.Bool(true))),
            ), text.toDelta()
        )
    }

    @Test
    fun TestDeltaApplyDelta3() {
        val doc = YDoc()
        val text = doc.getText("text")
        text.applyDelta(listOf(
            YInsert(YValue.String("hello"), null),
            YInsert(YValue.String("\n"), null),
        ))
        assertEquals(6u, text.length())

        text.applyDelta(listOf(
            YDelta.YRetain(6u, null),
            YInsert(YValue.String("hello2"), null),
            YInsert(YValue.String("\n"), null),
        ))
        assertEquals(13u, text.length())

        assertEquals(
            listOf(
                YInsert(YValue.String("hello\nhello2\n"), null),
            ), text.toDelta()
        )
    }

    @Test
    fun TestOffsetKind() {
        val doc = YDoc(YDocOptions(1u, gc = false))
        val text = doc.getText("text")
        text.applyDelta(listOf(
            YInsert(YValue.String(" – "), null),
        ))
        assertEquals(3u, text.length())

        val doc2 = YDoc(YDocOptions(1u, gc = false, bytesOffset = true))
        val text2 = doc2.getText("text")
        text2.applyDelta(listOf(
            YInsert(YValue.String(" – "), null),
        ))
        assertEquals(5u, text2.length())
    }

    @Test
    fun TestToString() {
        val doc = YDoc()
        val text = doc.getText("text")
        text.insert(0u, "hello")
        text.insert(5u, " world", "{\"bold\":true}")

        val delta = text.toText()
        assertEquals(
            "hello world", delta
        )
    }

    @Test
    fun TestInvalidOpTest() {
        val doc = YDoc()
        val text = doc.getText("text")
        text.insert(0u, "hello")

        assertThrows<InternalException> { text.delete(4u, 3u) }
        assertEquals("hell", text.toText())

        text.insert(5u, " world")
        assertEquals("hell world", text.toText())
    }
}