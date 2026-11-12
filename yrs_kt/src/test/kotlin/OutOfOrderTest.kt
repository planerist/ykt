import com.planerist.ykt.*
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * Updates out of order: a document that got a client's update before that client's earlier one, and
 * other clients' updates before the ones they build on, encoded whole or as a diff, in lib0 v1 and
 * v2. Every encoding reads on a fresh replica, which converges once the missing update arrives.
 */
class OutOfOrderTest {
    private fun doc(clientId: ULong) = YDoc(YDocOptions(clientId, gc = false))

    private fun edit(doc: YDoc, change: (YText, YTransaction) -> Unit): ByteArray {
        val text = doc.getText("t")
        return doc.transaction().use { txn ->
            change(text, txn)
            txn.encodeUpdateV2()
        }
    }

    /** A fresh replica that read `update`: it throws if the bytes don't read. */
    private fun replica(update: ByteArray, v2: Boolean): YDoc = doc(99u).also {
        if (v2) applyUpdateV2(it, update.toDirectByteBuffer()) else applyUpdate(it, update.toDirectByteBuffer())
    }

    @Test
    fun everyEncodingReadsAndConverges() {
        // Client 1: "abc", then "x" at the start, then "y" at the end (built on the first only).
        val source = doc(1u)
        val first = edit(source) { t, txn -> t.insert(0u, "abc", null, txn) }
        val second = edit(source) { t, txn -> t.insert(0u, "x", null, txn) }
        val third = edit(source) { t, txn -> t.insert(4u, "y", null, txn) }
        // Client 2 builds on the first: "B" after "a".
        val other = doc(2u).also { applyUpdateV2(it, first.toDirectByteBuffer()) }
        val fromOther = edit(other) { t, txn -> t.insert(1u, "B", null, txn) }
        // Client 3 builds on both: "C" after "B".
        val thirdClient = doc(3u).also { c -> listOf(first, fromOther).forEach { applyUpdateV2(c, it.toDirectByteBuffer()) } }
        val fromThird = edit(thirdClient) { t, txn -> t.insert(2u, "C", null, txn) }

        // The server gets them out of order — clients 2's and 3's before what they build on, client
        // 1's third before its second — and never the second.
        val server = doc(4u)
        val heard = Collections.synchronizedList(mutableListOf<ByteArray>())
        val sub = server.observeUpdateV2 { heard += it.update }
        for (update in listOf(fromThird, fromOther, third, first)) applyUpdateV2(server, update.toDirectByteBuffer())
        sub.close()

        val nothing = encodeStateVector(doc(98u))
        val encodings = listOf(
            encodeStateAsUpdateV2(server) to true,
            encodeStateAsUpdateV2(server, decodeStateVector(nothing)) to true,
            encodeStateAsUpdate(server) to false,
            encodeStateAsUpdate(server, decodeStateVector(nothing)) to false,
        )
        val more = listOf(
            server.transaction().use { it.diffV2(nothing) } to true,
            snapshot(server).use { encodeStateFromSnapshotV2(server, it) } to true,
        ) + heard.map { it to true }
        for ((bytes, v2) in encodings + more) replica(bytes, v2)

        // The whole state, then the missing update: everything.
        for ((bytes, v2) in encodings) {
            val replica = replica(bytes, v2)
            applyUpdateV2(replica, second.toDirectByteBuffer())
            assertEquals("xaBCbcy", replica.getText("t").toText())
        }
        applyUpdateV2(server, second.toDirectByteBuffer())
        assertEquals("xaBCbcy", server.getText("t").toText())
    }
}
