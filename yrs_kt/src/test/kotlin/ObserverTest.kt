import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Observers through the Kotlin binding: text deltas and origins, document updates and cleanups,
 * unsubscribing (twice, by close(), inside a callback, after the document is gone), what a callback
 * may not do (read, write, subscribe: it throws, it doesn't hang), and threads (a document shared
 * behind a lock; two replicas on two executors syncing through their update events).
 */
class ObserverTest {
    private fun str(text: String) = YValue.String(text)
    private fun insert(text: String) = YDelta.YInsert(str(text), null)
    private fun <T> synchronizedList(): MutableList<T> = Collections.synchronizedList(mutableListOf())

    // Text ----------------------------------------------------------------------------------------

    @Test
    fun insertFormatDeleteAsDeltas() {
        val doc = YDoc()
        val text = doc.getText("t")
        val deltas = synchronizedList<List<YDelta>>()
        text.observe { deltas += it.delta }.use {
            text.insert(0u, "hello")
            text.insert(5u, " world", "{\"bold\":true}")
            text.format(0u, 5u, "{\"italic\":true}")
            text.delete(1u, 3u)
        }

        assertEquals(
            listOf(
                listOf(insert("hello")),
                listOf(YDelta.YRetain(5u, null), YDelta.YInsert(str(" world"), mapOf("bold" to YValue.Bool(true)))),
                listOf(YDelta.YRetain(5u, mapOf("italic" to YValue.Bool(true)))),
                listOf(YDelta.YRetain(1u, null), YDelta.YDelete(3u)),
            ),
            deltas,
        )
    }

    @Test
    fun removingAnAttributeSendsNullAndOffsetsAreUtf16() {
        val doc = YDoc()
        val text = doc.getText("t")
        text.insert(0u, "😀a", "{\"bold\":true}")
        val deltas = synchronizedList<List<YDelta>>()
        text.observe { deltas += it.delta }.use {
            text.format(0u, 3u, "{\"bold\":null}")
            text.insert(3u, "b") // 😀 is two UTF-16 units
        }

        assertEquals(
            listOf(
                listOf(YDelta.YRetain(3u, mapOf("bold" to YValue.Null))),
                listOf(YDelta.YRetain(3u, null), insert("b")),
            ),
            deltas,
        )
    }

    @Test
    fun oneEventPerTransactionWithItsOrigin() {
        val doc = YDoc()
        val text = doc.getText("t")
        val events = synchronizedList<YTextEvent>()
        text.observe { events += it }.use {
            doc.transact("editor") { txn ->
                text.insert(0u, "ab", null, txn)
                text.insert(2u, "cd", null, txn)
                assertEquals(0, events.size, "nothing before the commit")
            }
            text.insert(0u, "x")
            doc.transact("empty") { } // no changes: no event
        }

        assertEquals(listOf(listOf(insert("abcd")), listOf(insert("x"))), events.map { it.delta })
        assertEquals(listOf("editor", null), events.map { it.originString })
    }

    @Test
    fun remoteUpdateWithItsOriginAndOnlyOnce() {
        val a = YDoc(YDocOptions(1u))
        a.getText("t").insert(0u, "hello")
        val b = YDoc(YDocOptions(2u))
        val text = b.getText("t")
        val events = synchronizedList<YTextEvent>()
        text.observe { events += it }.use {
            val update = encodeStateAsUpdateV2(a)
            applyUpdateV2(b, update.toDirectByteBuffer(), "server".toByteArray())
            applyUpdateV2(b, update.toDirectByteBuffer(), "server".toByteArray()) // nothing new
        }

        assertEquals(listOf(listOf(insert("hello"))), events.map { it.delta })
        assertEquals("server", events.single().originString)
    }

    @Test
    fun unsubscribeTwiceAndClose() {
        val doc = YDoc()
        val text = doc.getText("t")
        val first = synchronizedList<Int>()
        val second = synchronizedList<Int>()
        val firstSub = text.observe { first += 1 }
        val secondSub = text.observe { second += 1 }

        text.insert(0u, "a")
        assertTrue(firstSub.unsubscribe())
        assertFalse(firstSub.unsubscribe())
        assertFalse(firstSub.isActive())
        text.insert(1u, "b")
        secondSub.close() // released: unsubscribed
        text.insert(2u, "c")

        assertEquals(1, first.size)
        assertEquals(2, second.size)
        firstSub.close()
    }

    @Test
    fun subscriptionOutlivesTheDocument() {
        val heard = synchronizedList<Int>()
        val sub = run {
            val doc = YDoc()
            val text = doc.getText("t")
            val sub = text.observe { heard += 1 }
            text.insert(0u, "a")
            text.close()
            doc.close() // the document's handle is gone; the subscription keeps its store
            sub
        }
        assertTrue(sub.unsubscribe())
        sub.close()
        assertEquals(1, heard.size)
    }

    @Test
    fun readingWritingSubscribingInsideACallbackThrows() {
        val doc = YDoc()
        val text = doc.getText("t")
        val handle = doc.getText("t")
        val results = synchronizedList<String>()
        text.observe {
            results += if (runCatching { handle.toText() }.isFailure) "read throws" else "read worked"
            results += if (runCatching { handle.insert(0u, "x") }.isFailure) "write throws" else "write worked"
            results += if (runCatching { handle.observe { } }.isFailure) "subscribe throws" else "subscribe worked"
        }.use {
            text.insert(0u, "a")
        }

        assertEquals(listOf("read throws", "write throws", "subscribe throws"), results)
        assertEquals("a", text.toText())
    }

    @Test
    fun unsubscribeInsideItsOwnCallback() {
        val doc = YDoc()
        val text = doc.getText("t")
        val calls = synchronizedList<Int>()
        val updates = synchronizedList<Int>()
        var sub: YSubscription? = null
        var updateSub: YSubscription? = null
        sub = text.observe {
            calls += 1
            sub?.unsubscribe()
        }
        updateSub = doc.observeUpdateV2 {
            updates += 1
            updateSub?.unsubscribe() // the document is locked: switched off now, removed later
        }

        text.insert(0u, "a")
        text.insert(1u, "b")

        assertEquals(1, calls.size)
        assertEquals(1, updates.size)
        sub?.close()
        updateSub?.close()
    }

    @Test
    fun preliminaryTextCannotBeObserved() {
        val text = YText.newWithText("prelim")
        assertThrows<Exception> { text.observe { } }
    }

    // Document ------------------------------------------------------------------------------------

    @Test
    fun updatesReplayOnAnotherReplica() {
        for (v2 in listOf(false, true)) {
            val a = YDoc(YDocOptions(1u))
            val b = YDoc(YDocOptions(2u))
            val updates = synchronizedList<YUpdateEvent>()
            val text = a.getText("t")
            (if (v2) a.observeUpdateV2 { updates += it } else a.observeUpdateV1 { updates += it }).use {
                text.insert(0u, "hello")
                a.transact("editor") { txn ->
                    text.insert(5u, " world", null, txn)
                    text.delete(0u, 1u, txn)
                }
            }

            assertEquals(listOf(null, "editor"), updates.map { it.originString })
            for (event in updates) {
                if (v2) applyUpdateV2(b, event.update.toDirectByteBuffer()) else applyUpdate(b, event.update.toDirectByteBuffer())
            }
            assertEquals("ello world", b.getText("t").toText())
        }
    }

    @Test
    fun subscribingWhileATransactionIsOpenThrows() {
        val doc = YDoc()
        val txn = doc.transaction()
        assertThrows<Exception> { doc.observeUpdateV2 { } }
        txn.commit()
        doc.observeUpdateV2 { }.close()
    }

    @Test
    fun cleanupStateVectors() {
        val doc = YDoc(YDocOptions(7u))
        val text = doc.getText("t")
        val events = synchronizedList<YTransactionCleanupEvent>()
        doc.observeTransactionCleanup { events += it }.use {
            text.insert(0u, "hello")
            doc.transact("cut") { txn -> text.delete(0u, 2u, txn) }
            doc.transact("empty") { }
        }

        assertEquals(3, events.size, "every commit, an empty one too; not the subscription's own")
        assertEquals(5u, events[0].afterState[7uL])
        assertEquals(5u, events[1].beforeState[7uL])
        assertEquals("cut", events[1].originString)
        assertTrue(events[1].deleteSet.size > 1)
        assertEquals(events[2].beforeState, events[2].afterState)
    }

    // Threads -------------------------------------------------------------------------------------

    /** One call into a document at a time (ykt's contract), from many threads: a lock around it.
     *  Every callback runs on the thread that committed. */
    @Test
    fun sharedDocumentBehindALock() {
        val doc = YDoc()
        val text = doc.getText("t")
        val lock = Any()
        val callbackThreads = synchronizedList<String>()
        val workers = 8
        val perWorker = 50
        val start = CountDownLatch(1)
        val pool = Executors.newFixedThreadPool(workers) { r -> Thread(r) }
        val workerNames = Collections.synchronizedSet(mutableSetOf<String>())

        text.observe { callbackThreads += Thread.currentThread().name }.use {
            val jobs = (0 until workers).map {
                pool.submit {
                    workerNames += Thread.currentThread().name
                    start.await()
                    repeat(perWorker) { synchronized(lock) { text.insert(0u, "x") } }
                }
            }
            start.countDown()
            jobs.forEach { it.get(30, TimeUnit.SECONDS) }
        }
        pool.shutdown()

        assertEquals(workers * perWorker, callbackThreads.size)
        assertEquals(workerNames.toSet(), callbackThreads.toSet(), "each callback ran on a committing worker's thread")
        assertEquals((workers * perWorker).toUInt(), text.length())
    }

    /** Two replicas, each on a single-thread executor of its own (its only thread of access), edit at
     *  once and sync through their update events, as over a network: they converge. Updates applied
     *  from the other side have the origin "remote" and aren't sent back. */
    @Test
    fun twoReplicasOnTwoExecutorsConverge() {
        class Replica(client: ULong) {
            val executor = Executors.newSingleThreadExecutor()
            val doc = YDoc(YDocOptions(client))
            val text = doc.getText("t")
            lateinit var peer: Replica
            val subs = mutableListOf<YSubscription>()
            val heardRemote = synchronizedList<Int>()

            fun start() {
                subs += doc.observeUpdateV2 { event ->
                    if (event.originString != "remote") {
                        val update = event.update
                        peer.executor.submit { applyUpdateV2(peer.doc, update.toDirectByteBuffer(), "remote".toByteArray()) }
                    }
                }
                subs += text.observe { if (it.originString == "remote") heardRemote += 1 }
            }
        }

        val a = Replica(1u)
        val b = Replica(2u)
        a.peer = b
        b.peer = a
        a.executor.submit { a.start() }.get()
        b.executor.submit { b.start() }.get()
        val edits = 200
        val jobs = listOf(a to "a", b to "b").map { (replica, letter) ->
            replica.executor.submit { repeat(edits) { replica.text.insert(0u, letter) } }
        }
        jobs.forEach { it.get(30, TimeUnit.SECONDS) }
        // What each side sent is queued on the other: let both executors drain.
        repeat(2) {
            a.executor.submit { }.get()
            b.executor.submit { }.get()
        }

        val textA = a.executor.submit<String> { a.text.toText() }.get()
        val textB = b.executor.submit<String> { b.text.toText() }.get()
        assertEquals(textA, textB, "the replicas converge")
        assertEquals(2 * edits, textA.length)
        assertTrue(a.heardRemote.isNotEmpty() && b.heardRemote.isNotEmpty())
        for (replica in listOf(a, b)) {
            replica.executor.submit { replica.subs.forEach { it.close() } }.get()
            replica.executor.shutdown()
        }
    }
}
