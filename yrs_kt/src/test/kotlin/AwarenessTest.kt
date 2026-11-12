import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.util.Collections
import java.util.concurrent.atomic.AtomicLong
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Awareness: two peers exchanging states, observers with added / updated / removed clients and the
 * origin, removal by clearing, by removeStates and by the outdated check on an injected clock, what
 * callbacks may not do, garbage.
 */
class AwarenessTest {
    private class TestClock : YAwarenessClock {
        private val now = AtomicLong(0)
        override fun nowMillis(): ULong = now.get().toULong()
        fun advance(millis: Long) { now.addAndGet(millis) }
    }

    private fun peer(client: ULong, clock: TestClock) = YAwareness(YDoc(YDocOptions(client)), clock)
    private fun send(from: YAwareness, to: YAwareness) = to.applyUpdate(from.encodeUpdate(), "peer".toByteArray())
    private val ann = YValue.AttrMap(mapOf("name" to YValue.String("Ann"), "cursor" to YValue.AttrMap(mapOf("index" to YValue.Number(3.0)))))

    @Test
    fun twoPeersExchangeStates() {
        val clock = TestClock()
        val a = peer(1u, clock)
        val b = peer(2u, clock)
        a.setLocalState(ann)
        b.setLocalStateJson("{\"name\":\"Bob\"}")
        send(a, b)
        send(b, a)
        for (p in listOf(a, b)) {
            assertEquals(mapOf(1uL to ann, 2uL to YValue.AttrMap(mapOf("name" to YValue.String("Bob")))), p.states())
        }
        assertEquals("{\"name\":\"Bob\"}", b.statesJson()[2uL])
        assertEquals(1u, b.meta(1uL)?.clock)
    }

    @Test
    fun observersHearClientsAndTheOrigin() {
        val clock = TestClock()
        val a = peer(1u, clock)
        val b = peer(2u, clock)
        val changes = Collections.synchronizedList(mutableListOf<YAwarenessEvent>())
        b.observeChange { changes += it }.use {
            a.setLocalState(ann)
            send(a, b)
            a.setLocalState(YValue.AttrMap(mapOf("name" to YValue.String("Ann K."))))
            send(a, b)
            a.clearLocalState()
            send(a, b)
        }
        assertEquals(
            listOf(listOf(listOf(1uL), listOf(), listOf()), listOf(listOf(), listOf(1uL), listOf()), listOf(listOf(), listOf(), listOf(1uL))),
            changes.map { listOf(it.added, it.updated, it.removed) },
        )
        assertEquals(listOf("peer", "peer", "peer"), changes.map { it.originString })
        assertTrue(b.states().isEmpty())
    }

    @Test
    fun outdatedPeersAreRemoved() {
        val clock = TestClock()
        val a = peer(1u, clock)
        val b = peer(2u, clock)
        a.setLocalState(ann)
        b.setLocalState(YValue.Bool(true))
        send(a, b)
        clock.advance(20_000)
        assertEquals(listOf(), b.removeOutdated(30_000uL))
        clock.advance(15_000)
        assertEquals(listOf(1uL), b.removeOutdated(30_000uL))
        assertEquals(mapOf(2uL to YValue.Bool(true)), b.states())
        b.removeStates(listOf(2uL))
        assertNull(b.localState())
    }

    @Test
    fun insideACallbackItIsLocked() {
        val a = peer(1u, TestClock())
        val results = Collections.synchronizedList(mutableListOf<String>())
        a.observeChange {
            results += runCatching { a.states() }.exceptionOrNull()?.let { it::class.simpleName } ?: "worked"
        }.use { a.setLocalState(ann) }
        assertEquals(listOf("Locked"), results)
    }

    @Test
    fun garbageIsInvalidData() {
        val b = peer(2u, TestClock())
        assertThrows<YException.InvalidData> { b.applyUpdate(byteArrayOf(-1, -1, -1)) }
        assertThrows<YException.InvalidData> { b.setLocalStateJson("{nope") }
        assertTrue(b.states().isEmpty())
    }
}
