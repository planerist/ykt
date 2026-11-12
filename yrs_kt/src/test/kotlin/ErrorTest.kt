import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * YException, one case at a time: where each one is thrown, that nothing changed, and its message.
 * Also what used to crash or misbehave: an index past the end, a delta deleting past the end, a
 * handle used inside its own callback, a transaction used after its commit or of another document.
 */
class ErrorTest {
    private fun hello(gc: Boolean = true): Pair<YDoc, YText> {
        val doc = YDoc(YDocOptions(1u, gc = gc))
        val text = doc.getText("t")
        text.insert(0u, "hello")
        return doc to text
    }

    @Test
    fun lockedWhileATransactionIsOpen() {
        val (doc, text) = hello()
        doc.transact {
            assertThrows<YException.Locked> { text.toText() }
            assertThrows<YException.Locked> { doc.transaction() }
            assertThrows<YException.Locked> { snapshot(doc) }
            assertThrows<YException.Locked> { encodeStateVector(doc) }
        }
        assertEquals("hello", text.toText())
    }

    @Test
    fun theSameHandleInsideItsOwnCallbackIsLocked() {
        val (_, text) = hello()
        val results = Collections.synchronizedList(mutableListOf<String>())
        text.observe {
            for (body in listOf({ text.toText() }, { text.insert(0u, "x") })) {
                results += try { body(); "worked" } catch (e: YException) { e::class.simpleName!! }
            }
        }.use { text.insert(5u, "!") }
        assertEquals(listOf("Locked", "Locked"), results)
        assertEquals("hello!", text.toText())
    }

    @Test
    fun aCommittedTransactionThrowsEverywhere() {
        val (doc, text) = hello()
        val txn = doc.transaction()
        txn.commit()
        assertThrows<YException.TransactionCommitted> { txn.commit() }
        assertThrows<YException.TransactionCommitted> { txn.beforeState() }
        assertThrows<YException.TransactionCommitted> { txn.origin() }
        assertThrows<YException.TransactionCommitted> { txn.stateVectorV1() }
        assertThrows<YException.TransactionCommitted> { text.alive(txn) }
        assertThrows<YException.TransactionCommitted> { text.insert(0u, "x", null, txn) }
    }

    @Test
    fun anotherDocumentsTransactionIsTheWrongDocument() {
        val (a, text) = hello()
        val b = YDoc(YDocOptions(2u))
        val bText = b.getText("t")
        b.transact { txn ->
            assertThrows<YException.WrongDocument> { text.insert(0u, "x", null, txn) }
            assertThrows<YException.WrongDocument> { a.getMap("m").set("k", YValue.Null, txn) }
        }
        assertThrows<YException.WrongDocument> { YUndoManager(a, bText) }
        assertEquals("", bText.toText())
        assertEquals("hello", text.toText())
    }

    @Test
    fun aDeletedNodeIsTypeDeleted() {
        val doc = YDoc()
        val fragment = doc.getXmlFragment("x")
        val element = YXmlElement("p")
        fragment.push(YXmlChild.Element(element))
        fragment.delete(0u, null)
        assertThrows<YException.TypeDeleted> { element.length() }
        doc.transact { txn -> assertFalse(element.alive(txn)) }
    }

    @Test
    fun preliminaryAndAlreadyIntegrated() {
        val prelim = YText.newWithText("abc")
        assertThrows<YException.NotIntegrated> { prelim.format(0u, 1u, "{\"bold\":true}") }
        assertThrows<YException.NotIntegrated> { prelim.toDelta() }
        assertThrows<YException.NotIntegrated> { prelim.observe { } }

        val fragment = YDoc().getXmlFragment("x")
        val element = YXmlElement("p")
        fragment.push(YXmlChild.Element(element))
        assertThrows<YException.AlreadyIntegrated> { fragment.push(YXmlChild.Element(element)) }
        assertEquals(1u, fragment.length())
    }

    @Test
    fun pastTheEndIsOutOfBoundsAndChangesNothing() {
        val (_, text) = hello()
        assertThrows<YException.OutOfBounds> { text.insert(6u, "x") }
        assertThrows<YException.OutOfBounds> { text.delete(3u, 3u) }
        assertThrows<YException.OutOfBounds> { text.format(4u, 2u, "{\"bold\":true}") }
        assertThrows<YException.OutOfBounds> { text.applyDelta(listOf(YDelta.YRetain(2u, null), YDelta.YDelete(6u))) }
        assertEquals("hello", text.toText())

        val prelim = YText.newWithText("😀a")
        assertEquals(3u, prelim.length(), "UTF-16 units, as an integrated text")
        assertThrows<YException.OutOfBounds> { prelim.insert(1u, "x") }

        val fragment = YDoc().getXmlFragment("x")
        assertThrows<YException.OutOfBounds> { fragment.insert(1u, YXmlChild.Element(YXmlElement("p"))) }
        assertThrows<YException.OutOfBounds> { fragment.delete(0u, null) }
    }

    @Test
    fun attributesThatAreNotAnObjectAreInvalid() {
        val (_, text) = hello()
        for (bad in listOf("[1]", "not json")) {
            assertThrows<YException.InvalidAttributes> { text.insert(0u, "x", bad) }
        }
        assertEquals("hello", text.toText())
    }

    @Test
    fun malformedBytesAreInvalidData() {
        val (doc, _) = hello()
        val update = encodeStateAsUpdateV2(doc)
        for (cut in 1 until update.size) {
            try {
                applyUpdateV2(YDoc(), update.copyOf(cut).toDirectByteBuffer())
            } catch (e: YException) {
                assertTrue(e is YException.InvalidData, "$e")
            }
        }
        assertThrows<YException.InvalidData> { applyUpdate(doc, ByteArray(9) { 0xff.toByte() }.toDirectByteBuffer()) }
        assertThrows<YException.InvalidData> { decodeSnapshotV1(ByteArray(9) { 0xff.toByte() }.toDirectByteBuffer()) }
    }

    @Test
    fun snapshotsOfADocumentWithGcAreGcEnabled() {
        val (doc, text) = hello()
        val before = snapshot(doc)
        text.insert(5u, "!")
        assertThrows<YException.GcEnabled> { encodeStateFromSnapshotV2(doc, before) }
    }

    @Test
    fun messagesSayWhatHappened() {
        val (doc, text) = hello()
        doc.transact {
            val e = assertThrows<YException.Locked> { text.toText() }
            assertEquals("the document is locked: a transaction is open or a callback is running", e.message)
        }
        val e = assertThrows<YException.InvalidAttributes> { text.insert(0u, "x", "[1]") }
        assertEquals("formatting attributes must be a JSON object: not an object: [1]", e.message)
    }
}
