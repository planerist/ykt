import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * YMap and YArray: reads and writes, values of every kind (locally and on a replica), out-of-range
 * errors, concurrent edits, observers with origins, remote updates and unsubscribing, transactions.
 */
class MapArrayTest {
    private fun sync(from: YDoc, to: YDoc, origin: String? = null) =
        applyUpdateV2(to, encodeStateAsUpdateV2(from).toDirectByteBuffer(), origin?.toByteArray())

    private fun <T> synchronizedList(): MutableList<T> = Collections.synchronizedList(mutableListOf())
    private fun str(s: String) = YValue.String(s)
    private fun num(n: Double) = YValue.Number(n)

    private val everyKind: List<Pair<String, YValue>> = listOf(
        "null" to YValue.Null,
        "bool" to YValue.Bool(true),
        "int" to num(42.0),
        "float" to num(-1.5),
        "bigint" to YValue.BigInt((1L shl 60) + 1),
        "string" to str("héllo 😀"),
        "array" to YValue.Array(listOf(num(1.0), str("two"), YValue.Null, YValue.Array(listOf(YValue.Bool(false))))),
        "map" to YValue.AttrMap(mapOf("a" to num(1.0), "b" to YValue.AttrMap(emptyMap()))),
    )

    // YMap ---------------------------------------------------------------------------------------

    @Test
    fun mapSetGetDelete() {
        val doc = YDoc()
        val map = doc.getMap("m")
        assertNull(map.get("a"))

        map.set("a", num(1.0))
        map.set("b", str("two"))
        map.set("a", num(3.0))

        assertEquals(num(3.0), map.get("a"))
        assertTrue(map.containsKey("b"))
        assertFalse(map.containsKey("c"))
        assertEquals(2u, map.len())
        assertEquals(setOf("a", "b"), map.keys().toSet())
        assertEquals(map.toJson(), map.keys().zip(map.values()).toMap())
        assertEquals(mapOf("a" to num(3.0), "b" to str("two")), map.toJson())
        assertEquals(num(3.0), map.delete("a"))
        assertNull(map.delete("a"))
        map.clear()
        assertEquals(0u, map.len())
    }

    @Test
    fun mapValuesOfEveryKindOnAReplica() {
        val a = YDoc(YDocOptions(1u))
        for ((key, value) in everyKind) a.getMap("m").set(key, value)
        // Buffers are compared by content in a separate check: ByteArray has no value equality.
        a.getMap("m").set("buffer", YValue.Buffer(byteArrayOf(0, 1, -1)))
        val b = YDoc(YDocOptions(2u))
        sync(a, b)

        for (doc in listOf(a, b)) {
            for ((key, value) in everyKind) assertEquals(value, doc.getMap("m").get(key), key)
            val buffer = doc.getMap("m").get("buffer") as YValue.Buffer
            assertTrue(buffer.v1.contentEquals(byteArrayOf(0, 1, -1)))
        }
    }

    @Test
    fun mapObserverKeysOriginsAndRemote() {
        val doc = YDoc(YDocOptions(1u))
        val map = doc.getMap("m")
        map.set("kept", num(1.0))
        map.set("gone", num(2.0))
        val events = synchronizedList<YMapEvent>()
        map.observe { events += it }.use {
            doc.transact("editor") { txn ->
                map.set("new", str("y"), txn)
                map.set("kept", num(10.0), txn)
                map.delete("gone", txn)
            }
            val remote = YDoc(YDocOptions(2u))
            sync(doc, remote)
            remote.getMap("m").set("remote", YValue.Bool(true))
            sync(remote, doc, "server")
        }
        map.set("unheard", YValue.Null)

        assertEquals(
            listOf(
                mapOf(
                    "new" to YEntryChange.Inserted(str("y")),
                    "kept" to YEntryChange.Updated(num(1.0), num(10.0)),
                    "gone" to YEntryChange.Removed(num(2.0)),
                ),
                mapOf("remote" to YEntryChange.Inserted(YValue.Bool(true))),
            ),
            events.map { it.keys },
        )
        assertEquals(listOf("editor", "server"), events.map { it.originString })
    }

    // YArray -------------------------------------------------------------------------------------

    @Test
    fun arrayInsertPushGetDelete() {
        val doc = YDoc()
        val array = doc.getArray("a")
        array.push(listOf(num(1.0), num(2.0)))
        array.insert(0u, listOf(str("first")))
        array.insert(3u, listOf(str("last")))

        assertEquals(listOf(str("first"), num(1.0), num(2.0), str("last")), array.toJson())
        assertEquals(str("last"), array.get(3u))
        assertEquals(4u, array.len())
        array.delete(1u, 2u)
        assertEquals(listOf(str("first"), str("last")), array.toJson())

        assertThrows<Exception.OutOfBounds> { array.insert(3u, listOf(YValue.Null)) }
        assertThrows<Exception.OutOfBounds> { array.delete(1u, 2u) }
        assertThrows<Exception.OutOfBounds> { array.get(2u) }
        assertEquals(2u, array.len())
    }

    @Test
    fun arrayValuesOfEveryKindOnAReplica() {
        val a = YDoc(YDocOptions(1u))
        val values = everyKind.map { it.second }
        a.getArray("a").push(values)
        val b = YDoc(YDocOptions(2u))
        sync(a, b)

        assertEquals(values, a.getArray("a").toJson())
        assertEquals(values, b.getArray("a").toJson())
    }

    @Test
    fun concurrentArrayInsertsConverge() {
        val a = YDoc(YDocOptions(1u))
        val b = YDoc(YDocOptions(2u))
        a.getArray("a").push(listOf(str("base")))
        sync(a, b)
        a.getArray("a").insert(0u, listOf(str("a")))
        b.getArray("a").push(listOf(str("b")))

        sync(a, b)
        sync(b, a)

        assertEquals(a.getArray("a").toJson(), b.getArray("a").toJson())
        assertEquals(3u, a.getArray("a").len())
    }

    @Test
    fun arrayObserverDeltasAndOrigins() {
        val doc = YDoc()
        val array = doc.getArray("a")
        val events = synchronizedList<YArrayEvent>()
        val sub = array.observe { events += it }

        array.push(listOf(num(1.0), num(2.0), num(3.0)))
        array.insert(1u, listOf(str("x")))
        doc.transact("editor") { txn ->
            array.delete(0u, 1u, txn)
            array.push(listOf(YValue.Bool(true)), txn)
        }
        sub.close() // released: unsubscribed
        array.push(listOf(YValue.Null))

        assertEquals(
            listOf(
                listOf(YArrayDelta.Insert(listOf(num(1.0), num(2.0), num(3.0)))),
                listOf(YArrayDelta.Retain(1u), YArrayDelta.Insert(listOf(str("x")))),
                listOf(YArrayDelta.Delete(1u), YArrayDelta.Retain(3u), YArrayDelta.Insert(listOf(YValue.Bool(true)))),
            ),
            events.map { it.delta },
        )
        assertEquals(listOf(null, null, "editor"), events.map { it.originString })
    }

    // Transactions -------------------------------------------------------------------------------

    @Test
    fun insideATransactionAndACallback() {
        val doc = YDoc()
        val map = doc.getMap("m")
        val array = doc.getArray("a")
        doc.transact { txn ->
            map.set("k", num(1.0), txn)
            array.push(listOf(num(1.0)), txn)
            assertEquals(num(1.0), map.get("k", txn))
            assertEquals(1u, array.len(txn))
            assertThrows<Exception.AnotherRwTx> { map.get("k") }
            assertThrows<Exception.AnotherTx> { doc.getMap("m") }
        }

        val results = synchronizedList<Boolean>()
        val handle = doc.getMap("m")
        map.observe { results += runCatching { handle.get("k") }.isFailure }.use {
            map.set("k", num(2.0))
        }
        assertEquals(listOf(true), results, "reading in a callback throws")
    }
}
