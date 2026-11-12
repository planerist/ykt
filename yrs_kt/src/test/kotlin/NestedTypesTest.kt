import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Shared types nested in maps, arrays and texts: handles from `setText` / `setMap` / `setArray`,
 * `insertText` / `insertMap`, `insertEmbedMap`, read back with `getOutput`; edits replicating;
 * observers reporting handles; ids; undo over a map.
 */
class NestedTypesTest {
    private fun str(s: String) = YValue.String(s)
    private fun sync(from: YDoc, to: YDoc) {
        val missing = diffUpdateV2(encodeStateAsUpdateV2(from), encodeStateVector(to))
        applyUpdateV2(to, missing.toDirectByteBuffer())
    }

    @Test
    fun mapHandlesReplicateBothWays() {
        val a = YDoc(YDocOptions(1u))
        val root = a.getMap("root")
        val title = root.setText("title", "Hello")
        val meta = root.setMap("meta", mapOf("pinned" to YValue.Bool(true)))
        val tags = root.setArray("tags", listOf(str("work")))
        title.insert(5u, ", world")
        meta.set("color", str("red"))
        tags.push(listOf(str("home")))

        val b = YDoc(YDocOptions(2u))
        sync(a, b)
        val replica = b.getMap("root")
        val bTitle = replica.getOutput("title")!!.asText!!
        assertEquals("Hello, world", bTitle.toText())
        assertEquals(mapOf("pinned" to YValue.Bool(true), "color" to str("red")), replica.getOutput("meta")?.asMap?.toJson())
        assertEquals(listOf(str("work"), str("home")), replica.getOutput("tags")?.asArray?.toJson())
        assertEquals(str("Hello, world"), replica.get("title"), "the plain API gives JSON")

        bTitle.insert(0u, "> ")
        sync(b, a)
        assertEquals("> Hello, world", title.toText())
        assertEquals(YOutput.Text(title), root.getOutput("title"), "handles of one text are equal")
        assertEquals(title.id(), bTitle.id())
    }

    @Test
    fun arrayHandles() {
        val list = YDoc().getArray("list")
        val text = list.insertText(0u, "first")
        val map = list.insertMap(1u)
        map.set("k", YValue.Number(1.0))
        assertThrows<YException.OutOfBounds> { list.insertArray(5u) }
        assertEquals(text, list.getOutput(0u).asText)
        assertEquals(mapOf("k" to YValue.Number(1.0)), list.getOutput(1u).asMap?.toJson())
        assertEquals(listOf(str("first"), YValue.AttrMap(mapOf("k" to YValue.Number(1.0)))), list.toJson())
    }

    @Test
    fun observersReportHandles() {
        val root = YDoc().getMap("root")
        val events = Collections.synchronizedList(mutableListOf<YMapEvent>())
        val meta = root.observe { events += it }.use {
            val meta = root.setMap("meta")
            root.set("meta", str("replaced"))
            meta
        }
        assertEquals(
            listOf(
                mapOf("meta" to YEntryChange.Inserted(YOutput.Map(meta))),
                mapOf("meta" to YEntryChange.Updated(YOutput.Map(meta), YOutput.Value(str("replaced")))),
            ),
            events.map { it.keys },
        )
        assertThrows<YException.TypeDeleted> { meta.set("k", YValue.Null) }
    }

    @Test
    fun embeddedMapInAText() {
        val text = YDoc().getText("t")
        text.insert(0u, "ab")
        val table = text.insertEmbedMap(1u, mapOf("rows" to YValue.Number(2.0)))
        table.set("cols", YValue.Number(3.0))
        assertEquals(
            listOf(
                YDelta.YInsert(str("a"), null),
                YDelta.YInsert(YValue.AttrMap(mapOf("rows" to YValue.Number(2.0), "cols" to YValue.Number(3.0))), null),
                YDelta.YInsert(str("b"), null),
            ),
            text.toDelta(),
        )
    }

    @Test
    fun undoOverAMap() {
        val doc = YDoc()
        val root = doc.getMap("root")
        val undo = YUndoManager.forMap(doc, root, YUndoOptions(0uL, listOf("editor")))
        doc.transact("editor") { txn -> root.set("title", str("Draft"), txn) }
        val body = root.setText("body") // untracked
        doc.transact("editor") { txn -> body.insert(0u, "hello", null, txn) }

        assertTrue(undo.undo())
        assertEquals("", body.toText())
        assertTrue(undo.undo())
        assertEquals(null, root.get("title"))
        assertTrue(root.containsKey("body"))
        assertTrue(undo.redo())
        assertEquals(str("Draft"), root.get("title"))
    }
}
