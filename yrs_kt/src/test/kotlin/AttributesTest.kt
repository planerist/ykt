import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * Typed attributes, embeds of every value kind and the text's id: round trips through `toDelta`, an
 * observer and a replica; removal by `YValue.Null`; preliminary texts.
 */
class AttributesTest {
    private fun str(s: String) = YValue.String(s)
    private fun replicaOf(doc: YDoc): YDoc {
        val replica = YDoc(YDocOptions(99u))
        applyUpdateV2(replica, encodeStateAsUpdateV2(doc).toDirectByteBuffer())
        return replica
    }

    private val rich = mapOf(
        "bold" to YValue.Bool(true),
        "color" to str("#ff0000"),
        "size" to YValue.Number(14.0),
        "link" to YValue.AttrMap(mapOf("href" to str("https://yaranga.net"))),
        "tags" to YValue.Array(listOf(str("a"), YValue.Null)),
    )

    @Test
    fun roundTripThroughDeltaObserverAndReplica() {
        val doc = YDoc(YDocOptions(1u))
        val text = doc.getText("t")
        val deltas = Collections.synchronizedList(mutableListOf<List<YDelta>>())
        text.observe { deltas += it.delta }.use {
            text.insertWithAttributes(0u, "hello", rich)
            text.insertWithAttributes(5u, " world", emptyMap()) // plain, not the formatting before it
            text.formatWithAttributes(6u, 5u, mapOf("italic" to YValue.Bool(true)))
        }

        val expected = listOf(
            YDelta.YInsert(str("hello"), rich),
            YDelta.YInsert(str(" "), null),
            YDelta.YInsert(str("world"), mapOf("italic" to YValue.Bool(true))),
        )
        assertEquals(expected, text.toDelta())
        assertEquals(expected, replicaOf(doc).getText("t").toDelta())
        assertEquals(
            listOf(
                listOf(YDelta.YInsert(str("hello"), rich)),
                listOf(YDelta.YRetain(5u, null), YDelta.YInsert(str(" world"), null)),
                listOf(YDelta.YRetain(6u, null), YDelta.YRetain(5u, mapOf("italic" to YValue.Bool(true)))),
            ),
            deltas,
        )
    }

    @Test
    fun nullRemovesAnAttribute() {
        val text = YDoc().getText("t")
        text.insertWithAttributes(0u, "hello", mapOf("bold" to YValue.Bool(true), "italic" to YValue.Bool(true)))
        text.formatWithAttributes(0u, 2u, mapOf("bold" to YValue.Null))
        assertEquals(
            listOf(
                YDelta.YInsert(str("he"), mapOf("italic" to YValue.Bool(true))),
                YDelta.YInsert(str("llo"), mapOf("bold" to YValue.Bool(true), "italic" to YValue.Bool(true))),
            ),
            text.toDelta(),
        )
    }

    @Test
    fun embedsOfEveryKind() {
        val every = listOf(
            YValue.Null, YValue.Bool(false), YValue.Number(42.0), YValue.Number(-1.5), YValue.BigInt((1L shl 60) + 1),
            str("embedded"), YValue.Buffer(byteArrayOf(0, 1, -1)), YValue.Array(listOf(YValue.Number(1.0), str("two"))),
            YValue.AttrMap(mapOf("mention" to str("note://1"))),
        )
        val doc = YDoc(YDocOptions(1u))
        val text = doc.getText("t")
        text.insert(0u, "ab")
        every.forEachIndexed { i, value -> text.insertEmbed((1 + i).toUInt(), value) }
        text.insertEmbed(1u, YValue.Bool(true), mapOf("bold" to YValue.Bool(true)))

        val expected = listOf(YDelta.YInsert(str("a"), null), YDelta.YInsert(YValue.Bool(true), mapOf("bold" to YValue.Bool(true)))) +
            every.map { YDelta.YInsert(it, null) } + YDelta.YInsert(str("b"), null)
        assertEquals((2 + every.size + 1).toUInt(), text.length())
        assertDeltasEqual(expected, text.toDelta())
        assertDeltasEqual(expected, replicaOf(doc).getText("t").toDelta())
    }

    /** As assertEquals, comparing buffers by content (a ByteArray's equals is identity). */
    private fun assertDeltasEqual(expected: List<YDelta>, actual: List<YDelta>) {
        fun plain(v: YValue): Any = when (v) {
            is YValue.Buffer -> v.v1.toList()
            is YValue.Array -> v.v1.map(::plain)
            is YValue.AttrMap -> v.v1.mapValues { plain(it.value) }
            else -> v
        }
        fun plain(d: YDelta): Any = when (d) {
            is YDelta.YInsert -> listOf(plain(d.v1), d.v2)
            else -> d
        }
        assertEquals(expected.map(::plain), actual.map(::plain))
    }

    @Test
    fun pastTheEndAndPreliminary() {
        val text = YDoc().getText("t")
        text.insert(0u, "abc")
        assertThrows<YException.OutOfBounds> { text.insertWithAttributes(4u, "x", rich) }
        assertThrows<YException.OutOfBounds> { text.insertEmbed(4u, YValue.Null) }
        val prelim = YText.newWithText("abc")
        assertThrows<YException.NotIntegrated> { prelim.insertWithAttributes(0u, "x", rich) }
        assertThrows<YException.NotIntegrated> { prelim.formatWithAttributes(0u, 1u, rich) }
        assertThrows<YException.NotIntegrated> { prelim.insertEmbed(0u, YValue.Null) }
        assertThrows<YException.NotIntegrated> { prelim.id() }
    }

    @Test
    fun rootTextIdIsItsName() {
        val doc = YDoc(YDocOptions(1u))
        val text = doc.getText("lines")
        text.insert(0u, "x")
        assertEquals(YBranchId.Root("lines"), text.id())
        assertEquals(text.id(), replicaOf(doc).getText("lines").id())
    }
}
