import com.planerist.ykt.*
import com.planerist.ykt.YDelta.YInsert
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

        assertEquals(txt2.getText(), "hello")
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
        assertEquals(txt2.getText(), "hello")
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
}