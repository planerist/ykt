import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Update utilities without a document: merging, diffing against a replica's state vector, state
 * vectors read from updates, and bytes that aren't updates (an exception, never a crash).
 */
class UpdateTest {
    /** A document edited in five transactions, with every transaction's update as it was sent. */
    private fun edited(v2: Boolean): Pair<YDoc, List<ByteArray>> {
        val doc = YDoc(YDocOptions(1u))
        val updates = Collections.synchronizedList(mutableListOf<ByteArray>())
        val text = doc.getText("t")
        (if (v2) doc.observeUpdateV2 { updates += it.update } else doc.observeUpdateV1 { updates += it.update }).use {
            text.insert(0u, "hello")
            text.insert(5u, " world")
            text.format(0u, 5u, "{\"bold\":true}")
            text.delete(0u, 1u)
            text.insert(10u, "!")
        }
        return doc to updates.toList()
    }

    private fun apply(update: ByteArray, doc: YDoc, v2: Boolean) =
        if (v2) applyUpdateV2(doc, update.toDirectByteBuffer()) else applyUpdate(doc, update.toDirectByteBuffer())

    private fun full(doc: YDoc, v2: Boolean) = if (v2) encodeStateAsUpdateV2(doc) else encodeStateAsUpdate(doc)
    private fun merge(updates: List<ByteArray>, v2: Boolean) = if (v2) mergeUpdatesV2(updates) else mergeUpdatesV1(updates)
    private fun diff(update: ByteArray, sv: ByteArray, v2: Boolean) = if (v2) diffUpdateV2(update, sv) else diffUpdateV1(update, sv)
    private fun stateVectorOf(update: ByteArray, v2: Boolean) =
        if (v2) encodeStateVectorFromUpdateV2(update) else encodeStateVectorFromUpdateV1(update)

    @Test
    fun mergingIsApplyingOneByOne() {
        for (v2 in listOf(false, true)) {
            val (source, updates) = edited(v2)
            assertEquals(5, updates.size)
            for (merged in listOf(merge(updates, v2), merge(updates.reversed(), v2))) {
                val target = YDoc(YDocOptions(2u))
                apply(merged, target, v2)
                assertEquals("ello world!", target.getText("t").toText())
                assertEquals(source.getText("t").toDelta(), target.getText("t").toDelta())
                assertContentEquals(encodeStateVector(source), encodeStateVector(target))
            }
        }
    }

    @Test
    fun mergingNothingIsAnEmptyUpdate() {
        for (v2 in listOf(false, true)) {
            val target = YDoc()
            val heard = Collections.synchronizedList(mutableListOf<Int>())
            target.observeUpdateV1 { heard += 1 }.use {
                apply(merge(emptyList(), v2), target, v2)
            }
            assertEquals(0, heard.size)
            assertEquals("", target.getText("t").toText())
        }
    }

    @Test
    fun aDiffIsWhatAReplicaLacks() {
        for (v2 in listOf(false, true)) {
            val (source, updates) = edited(v2)
            val all = full(source, v2)
            val replica = YDoc(YDocOptions(2u))
            apply(updates[0], replica, v2)
            apply(updates[1], replica, v2)

            val missing = diff(all, encodeStateVector(replica), v2)
            assertTrue(missing.size < all.size)
            val fresh = YDoc(YDocOptions(3u))
            apply(missing, fresh, v2)
            assertEquals("", fresh.getText("t").toText(), "alone it builds on nothing")

            apply(missing, replica, v2)
            assertEquals("ello world!", replica.getText("t").toText())
            assertEquals(source.getText("t").toDelta(), replica.getText("t").toDelta())
        }
    }

    @Test
    fun aDiffKeepsDeletions() {
        for (v2 in listOf(false, true)) {
            val a = YDoc(YDocOptions(1u))
            a.getText("t").insert(0u, "abc")
            val replica = YDoc(YDocOptions(2u))
            apply(full(a, v2), replica, v2)
            a.getText("t").delete(0u, 1u)

            apply(diff(full(a, v2), encodeStateVector(replica), v2), replica, v2)

            assertEquals("bc", replica.getText("t").toText())
        }
    }

    @Test
    fun theStateVectorOfAnUpdateIsTheDocuments() {
        for (v2 in listOf(false, true)) {
            val (source, updates) = edited(v2)
            assertContentEquals(encodeStateVector(source), stateVectorOf(full(source, v2), v2))
            // Clocks count from 0: one later transaction's update says nothing.
            assertContentEquals(encodeStateVector(YDoc()), stateVectorOf(updates[1], v2))
        }
    }

    @Test
    fun garbageThrows() {
        val (_, updates) = edited(false)
        for (bytes in listOf(byteArrayOf(), ByteArray(12) { 0xff.toByte() }, byteArrayOf(1, 2, 3))) {
            for (v2 in listOf(false, true)) {
                assertThrows<Exception.InvalidData> { merge(listOf(bytes), v2) }
                assertThrows<Exception.InvalidData> { diff(bytes, byteArrayOf(0), v2) }
                assertThrows<Exception.InvalidData> { stateVectorOf(bytes, v2) }
            }
        }
        assertThrows<Exception.InvalidData> { mergeUpdatesV1(listOf(updates[0], ByteArray(5) { 0xff.toByte() })) }
        assertThrows<Exception.InvalidData> { diffUpdateV1(updates[0], byteArrayOf(0xff.toByte(), 0xff.toByte())) }
        assertThrows<Exception.InvalidData>("a v1 update isn't v2") { mergeUpdatesV2(listOf(updates[0])) }
        // Every cut of a real update: a result or an exception, never a crash.
        val all = merge(edited(true).second, true)
        for (length in 0 until all.size) {
            runCatching { mergeUpdatesV2(listOf(all.copyOf(length))) }
            runCatching { encodeStateVectorFromUpdateV2(all.copyOf(length)) }
        }
    }
}
