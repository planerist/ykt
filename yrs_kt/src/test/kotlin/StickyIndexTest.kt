import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertNull

/**
 * Relative positions: assoc, local and remote edits, the start and the end, encoding (Yjs's bytes)
 * and other replicas, transactions, bytes that aren't a position.
 */
class StickyIndexTest {
    private fun textWith(doc: YDoc, content: String): YText = doc.getText("t").also { it.insert(0u, content) }
    private fun sync(from: YDoc, to: YDoc) = applyUpdateV2(to, encodeStateAsUpdateV2(from).toDirectByteBuffer())

    @Test
    fun afterMovesOnWithAnInsertAtItsSpotAndBeforeStays() {
        val doc = YDoc(YDocOptions(1u))
        val text = textWith(doc, "abc")
        val before = text.stickyIndex(1u, YAssoc.BEFORE)
        val after = text.stickyIndex(1u, YAssoc.AFTER)

        text.insert(1u, "XY")

        assertEquals(YStickyOffset(1u, YAssoc.BEFORE), before.offset(doc))
        assertEquals(YStickyOffset(3u, YAssoc.AFTER), after.offset(doc))
    }

    @Test
    fun localAndRemoteEdits() {
        val a = YDoc(YDocOptions(1u))
        val aText = textWith(a, "hello world")
        val b = YDoc(YDocOptions(2u))
        sync(a, b)
        val w = b.getText("t").stickyIndex(6u, YAssoc.AFTER) // before "w", made on B

        aText.insert(0u, ">> ")
        aText.delete(3u, 5u) // ">>  world"
        sync(a, b)
        assertEquals(4u, w.offset(b)?.index)

        b.getText("t").delete(1u, 5u) // ">rld": its neighbour is gone
        assertEquals(1u, w.offset(b)?.index)
    }

    @Test
    fun theStartAndTheEnd() {
        val doc = YDoc(YDocOptions(1u))
        val text = doc.getText("t")
        val start = text.stickyIndex(0u, YAssoc.BEFORE)
        val end = text.stickyIndex(0u, YAssoc.AFTER) // an empty text: the end

        text.insert(0u, "abc")
        val beforeC = text.stickyIndex(3u, YAssoc.BEFORE)
        text.insert(3u, "de")
        text.insert(0u, "X")

        assertEquals(0u, start.offset(doc)?.index)
        assertEquals(6u, end.offset(doc)?.index)
        assertEquals(4u, beforeC.offset(doc)?.index)
        assertThrows<YException.OutOfBounds> { text.stickyIndex(7u, YAssoc.AFTER) }
    }

    @Test
    fun utf16Offsets() {
        val doc = YDoc()
        val text = textWith(doc, "😀ab")
        val b = text.stickyIndex(3u, YAssoc.AFTER)
        text.insert(0u, "😀")
        assertEquals(5u, b.offset(doc)?.index)
    }

    @Test
    fun encodedAsYjsAndReadOnAnotherReplica() {
        val a = YDoc(YDocOptions(1u))
        val text = textWith(a, "abc")
        val b = text.stickyIndex(1u, YAssoc.AFTER)
        assertContentEquals(byteArrayOf(0, 1, 1, 0), b.encode(), "Y.encodeRelativePosition's bytes")
        assertContentEquals(byteArrayOf(1, 1, 't'.code.toByte(), 0), text.stickyIndex(3u, YAssoc.AFTER).encode())

        val received = YStickyIndex.decode(b.encode())
        assertEquals(b, received)
        assertEquals(b.hashCode(), received.hashCode())
        assertNotEquals(text.stickyIndex(1u, YAssoc.BEFORE), received)
        assertEquals(YAssoc.AFTER, received.assoc())
        val replica = YDoc(YDocOptions(2u))
        assertNull(received.offset(replica), "not there yet")
        sync(a, replica)
        assertEquals(1u, received.offset(replica)?.index)
        assertEquals("YStickyIndex(after 1#1)", received.toString())
    }

    @Test
    fun garbageThrows() {
        for (bytes in listOf(byteArrayOf(), ByteArray(6) { 0xff.toByte() }, byteArrayOf(9, 1, 1, 0), byteArrayOf(0, 1))) {
            assertThrows<YException.InvalidData> { YStickyIndex.decode(bytes) }
        }
        assertThrows<YException.NotIntegrated> { YText.newWithText("abc").stickyIndex(0u, YAssoc.AFTER) }
    }

    @Test
    fun insideATransaction() {
        val doc = YDoc()
        val text = textWith(doc, "abc")
        val c = text.stickyIndex(2u, YAssoc.AFTER)

        doc.transact { txn ->
            text.insert(0u, "XY", null, txn)
            assertEquals(4u, c.offset(doc, txn)?.index)
            assertEquals(2u, text.stickyIndex(2u, YAssoc.AFTER, txn).offset(doc, txn)?.index)
            assertThrows<YException.Locked> { c.offset(doc) }
        }
        assertEquals(4u, c.offset(doc)?.index)
    }
}
