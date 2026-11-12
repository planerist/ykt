import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.lang.ref.WeakReference
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * The undo manager with the web editor's setup (tracked origins = the editor's, so remote edits are
 * never undone), redo, the capture timeout, clearing, origins, stack item meta, what throws inside a
 * transaction or a callback, releasing it, and threads behind a lock.
 */
class UndoManagerTest {
    private fun manager(doc: YDoc, text: YText, timeout: ULong = 0uL, origins: List<String> = listOf("editor")) =
        YUndoManager(doc, text, YUndoOptions(timeout, origins))

    private fun insert(doc: YDoc, text: YText, origin: String, index: UInt, chunk: String) =
        doc.transact(origin) { txn -> text.insert(index, chunk, null, txn) }

    /** What `remote` has that `local` lacks, applied to `local` with `origin`. */
    private fun receive(local: YDoc, remote: YDoc, origin: String? = null) {
        val missing = diffUpdateV2(encodeStateAsUpdateV2(remote), encodeStateVector(local))
        applyUpdateV2(local, missing.toDirectByteBuffer(), origin?.toByteArray())
    }

    private fun <T> synchronizedList(): MutableList<T> = Collections.synchronizedList(mutableListOf())

    @Test
    fun undoTakesBackTheEditorsChangesAndKeepsTheRemoteOnes() {
        val local = YDoc(YDocOptions(1u))
        val text = local.getText("t")
        val remote = YDoc(YDocOptions(2u))
        manager(local, text).use { undo ->
            insert(local, text, "editor", 0u, "hello")
            receive(remote, local)
            remote.getText("t").insert(5u, " world")
            remote.getText("t").insert(2u, "~") // inside the editor's "hello"
            receive(local, remote, "remote")
            insert(local, text, "editor", 0u, "> ")
            assertEquals("> he~llo world", text.toText())

            assertTrue(undo.undo())
            assertEquals("he~llo world", text.toText())
            assertTrue(undo.undo())
            assertEquals("~ world", text.toText(), "only the editor's characters are gone")
            assertFalse(undo.canUndo())
            assertFalse(undo.undo())

            assertTrue(undo.redo())
            assertTrue(undo.redo())
            assertEquals("> he~llo world", text.toText())
            assertFalse(undo.canRedo())
        }
    }

    @Test
    fun captureTimeoutMergesAndStopCapturingSplits() {
        val doc = YDoc()
        val text = doc.getText("t")
        manager(doc, text, timeout = 3_600_000uL).use { undo ->
            insert(doc, text, "editor", 0u, "a")
            insert(doc, text, "editor", 1u, "b")
            undo.stopCapturing()
            insert(doc, text, "editor", 2u, "c")
            insert(doc, text, "editor", 3u, "d")

            assertTrue(undo.undo())
            assertEquals("ab", text.toText())
            assertTrue(undo.undo())
            assertEquals("", text.toText())
        }
    }

    @Test
    fun clearAndUntrackedOrigins() {
        val doc = YDoc()
        val text = doc.getText("t")
        manager(doc, text).use { undo ->
            insert(doc, text, "other", 0u, "a")
            text.insert(1u, "b") // no origin
            assertFalse(undo.canUndo())

            undo.addTrackedOrigin("other")
            insert(doc, text, "other", 2u, "c")
            undo.removeTrackedOrigin("editor")
            insert(doc, text, "editor", 3u, "d")
            assertTrue(undo.canUndo())
            undo.clear()
            assertFalse(undo.canUndo())
            assertFalse(undo.undo())
            assertEquals("abcd", text.toText())
        }
    }

    @Test
    fun defaultOptionsTrackChangesWithoutAnOrigin() {
        val doc = YDoc()
        val text = doc.getText("t")
        YUndoManager(doc, text).use { undo ->
            text.insert(0u, "a")
            insert(doc, text, "editor", 1u, "b")
            assertTrue(undo.undo())
            assertEquals("b", text.toText())
        }
    }

    @Test
    fun metaComesBackWhenTheItemIsPopped() {
        val doc = YDoc()
        val text = doc.getText("t")
        val undo = manager(doc, text)
        var selection = 0.0
        val added = synchronizedList<YUndoEvent>()
        val popped = synchronizedList<YUndoEvent>()
        val addedSub = undo.observeItemAdded { event ->
            added += event
            mapOf("selection" to YValue.Number(selection))
        }
        val poppedSub = undo.observeItemPopped { popped += it }

        selection = 1.0
        insert(doc, text, "editor", 0u, "a")
        selection = 9.0
        undo.undo()
        undo.redo()

        val own = undo.origin()
        assertEquals(listOf(YUndoKind.UNDO, YUndoKind.REDO, YUndoKind.UNDO), added.map { it.kind })
        assertEquals("editor", added.first().originString)
        assertEquals(emptyMap<String, YValue>(), added.first().meta)
        assertEquals(listOf(YUndoKind.UNDO, YUndoKind.REDO), popped.map { it.kind })
        popped.forEach { assertContentEquals(own, it.origin) }
        assertEquals(listOf<Map<String, YValue>>(mapOf("selection" to YValue.Number(1.0)), mapOf("selection" to YValue.Number(9.0))), popped.map { it.meta })
        addedSub.close()
        poppedSub.close()
        undo.close()
    }

    @Test
    fun anExtendedItemCanChangeItsMeta() {
        val doc = YDoc()
        val text = doc.getText("t")
        manager(doc, text, timeout = 3_600_000uL).use { undo ->
            val popped = synchronizedList<Map<String, YValue>>()
            val subs = listOf(
                undo.observeItemAdded { mapOf("n" to YValue.Number(1.0)) },
                undo.observeItemUpdated { it.meta + ("n" to YValue.Number(2.0)) },
                undo.observeItemPopped { popped += it.meta },
            )
            insert(doc, text, "editor", 0u, "a")
            insert(doc, text, "editor", 1u, "b")
            undo.undo()

            assertEquals(listOf<Map<String, YValue>>(mapOf("n" to YValue.Number(2.0))), popped)
            subs.forEach { it.close() }
        }
    }

    @Test
    fun undoTransactionsCarryTheManagersOrigin() {
        val doc = YDoc()
        val text = doc.getText("t")
        manager(doc, text).use { undo ->
            val origins = synchronizedList<ByteArray?>()
            text.observe { origins += it.origin }.use {
                insert(doc, text, "editor", 0u, "a")
                undo.undo()
            }
            assertEquals(2, origins.size)
            assertContentEquals("editor".toByteArray(), origins[0])
            assertContentEquals(undo.origin(), origins[1])
        }
    }

    @Test
    fun insideATransactionOrACallbackItThrows() {
        val doc = YDoc()
        val text = doc.getText("t")
        manager(doc, text).use { undo ->
            insert(doc, text, "editor", 0u, "a")
            doc.transact("editor") {
                assertThrows<Exception.AnotherTx> { undo.undo() }
                assertThrows<Exception.AnotherTx> { undo.redo() }
                assertThrows<Exception.AnotherTx> { undo.clear() }
                assertTrue(undo.canUndo())
            }

            val results = synchronizedList<String>()
            val textSub = text.observe {
                results += if (runCatching { undo.undo() }.isFailure) "undo throws in a text observer" else "undo worked"
            }
            val weak = WeakReference(undo)
            val poppedSub = undo.observeItemPopped {
                results += if (runCatching { weak.get()!!.canRedo() }.isFailure) "the manager throws in its callback" else "it worked"
            }
            insert(doc, text, "editor", 1u, "b")
            textSub.close()
            assertTrue(undo.undo())

            assertEquals(listOf("undo throws in a text observer", "the manager throws in its callback"), results)
            assertEquals("a", text.toText())
            poppedSub.close()
        }
    }

    @Test
    fun releasingTheManagerIsSafeAnywhere() {
        val doc = YDoc()
        val text = doc.getText("t")
        val undo = manager(doc, text)
        val sub = undo.observeItemAdded { null }
        insert(doc, text, "editor", 0u, "a")

        doc.transact("editor") { txn ->
            text.insert(1u, "b", null, txn)
            undo.close() // the document is locked: the manager stays behind, harmlessly
        }
        insert(doc, text, "editor", 2u, "c")

        assertEquals("abc", text.toText())
        assertTrue(sub.unsubscribe(), "a subscription may outlive its manager")
        sub.close()
    }

    /** One call at a time (ykt's contract), from many threads: a lock around the document and its manager. */
    @Test
    fun sharedBehindALock() {
        val doc = YDoc()
        val text = doc.getText("t")
        val lock = Any()
        val workers = 8
        val perWorker = 20
        val start = CountDownLatch(1)
        val pool = Executors.newFixedThreadPool(workers)
        manager(doc, text).use { undo ->
            val jobs = (0 until workers).map {
                pool.submit {
                    start.await()
                    repeat(perWorker) { synchronized(lock) { insert(doc, text, "editor", 0u, "x") } }
                }
            }
            start.countDown()
            jobs.forEach { it.get(30, TimeUnit.SECONDS) }

            var undone = 0
            while (pool.submit<Boolean> { synchronized(lock) { undo.undo() } }.get()) undone++
            assertEquals(workers * perWorker, undone)
            assertEquals("", text.toText())
        }
        pool.shutdown()
    }
}
