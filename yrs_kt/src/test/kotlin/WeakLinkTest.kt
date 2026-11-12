import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Weak links: to map entries (`YMap.link`) and quotes of array ranges (`YArray.quote`), inserted
 * into maps, arrays and texts, read with `unquote` / `deref` locally and on a replica after v2
 * updates; bounds and errors; observers, and callbacks that may not read the document.
 */
class WeakLinkTest {
    private fun str(s: String) = YValue.String(s)
    private fun num(n: Int) = YOutput.Value(YValue.Number(n.toDouble()))
    private fun out(s: String) = YOutput.Value(str(s))
    private fun numbers(list: YArray) = list.push((1..4).map { YValue.Number(it.toDouble()) })
    private fun sync(from: YDoc, to: YDoc) {
        val missing = diffUpdateV2(encodeStateAsUpdateV2(from), encodeStateVector(to))
        applyUpdateV2(to, missing.toDirectByteBuffer())
    }

    @Test
    fun mapLinkFollowsItsEntry() {
        val settings = YDoc().getMap("settings")
        settings.set("theme", str("dark"))
        val quote = settings.link("theme")!!
        assertTrue(quote.prelim())
        assertThrows<YException.NotIntegrated> { quote.deref() }

        val current = settings.setWeakLink("current", quote)
        assertEquals(out("dark"), current.deref())
        settings.set("theme", str("light"))
        assertEquals(out("light"), current.deref(), "the entry's value now")
        assertEquals(current, settings.getOutput("current")?.asWeakLink)
        assertEquals(YValue.Null, settings.get("current"), "the plain API gives its JSON form")

        settings.delete("theme")
        assertNull(current.deref())
        assertEquals(emptyList(), current.unquote())
        assertNull(settings.link("theme"), "no entry to link")
    }

    @Test
    fun arrayQuoteAcrossReplicas() {
        val a = YDoc(YDocOptions(1u))
        val list = a.getArray("list")
        numbers(list)
        val link = a.getMap("notes").setWeakLink("q", list.quote(1u, 2u))
        list.insert(2u, listOf(str("inside")))
        list.insert(1u, listOf(str("before"))) // at its edge: not quoted
        assertEquals(listOf(num(2), out("inside"), num(3)), link.unquote())
        assertEquals(num(2), link.deref(), "a quote's first value")

        val b = YDoc(YDocOptions(2u))
        sync(a, b)
        val replica = b.getMap("notes").getOutput("q")!!.asWeakLink!!
        assertEquals(link.unquote(), replica.unquote())
        assertEquals(link.id(), replica.id())

        b.getArray("list").insert(3u, listOf(str("remote"))) // between 2 and "inside"
        sync(b, a)
        assertEquals(listOf(num(2), out("remote"), out("inside"), num(3)), link.unquote())
    }

    @Test
    fun insertingLinksBoundsAndErrors() {
        val a = YDoc(YDocOptions(1u))
        val list = a.getArray("list")
        assertThrows<YException.OutOfBounds> { list.quote(0u, 1u) }
        numbers(list)
        assertThrows<YException.OutOfBounds> { list.quote(1u, 0u) }
        assertThrows<YException.OutOfBounds> { list.quote(3u, 2u) }

        val quote = list.quote(0u, 2u)
        val refs = a.getArray("refs")
        val inArray = refs.insertWeakLink(0u, quote)
        val text = a.getText("t")
        text.insert(0u, "see ")
        val inText = text.insertEmbedWeakLink(4u, quote, mapOf("bold" to YValue.Bool(true)))
        val copy = a.getMap("notes").setWeakLink("copy", inText) // an integrated link is copied
        for (link in listOf(inArray, inText, copy)) assertEquals(listOf(num(1), num(2)), link.unquote())
        assertEquals(YDelta.YInsert(YValue.Null, mapOf("bold" to YValue.Bool(true))), text.toDelta()[1])

        assertThrows<YException.OutOfBounds> { refs.insertWeakLink(2u, quote) }
        assertThrows<YException.WrongDocument> { YDoc().getMap("m").setWeakLink("k", quote) }
        refs.delete(0u, 1u)
        assertThrows<YException.TypeDeleted> { refs.insertWeakLink(0u, inArray) }
    }

    @Test
    fun observersAndLockedCallbacks() {
        val doc = YDoc()
        val list = doc.getArray("list")
        numbers(list)
        val notes = doc.getMap("notes")
        val link = notes.setWeakLink("q", list.quote(0u, 2u))
        val events = Collections.synchronizedList(mutableListOf<YWeakLinkEvent>())
        val locked = Collections.synchronizedList(mutableListOf<Boolean>())
        link.observe {
            events += it
            locked += runCatching { link.unquote() }.exceptionOrNull() is YException.Locked
            locked += runCatching { list.quote(0u, 1u) }.exceptionOrNull() is YException.Locked
            locked += runCatching { notes.setWeakLink("k", link) }.exceptionOrNull() is YException.Locked
        }.use {
            doc.transact("editor") { txn -> list.insert(1u, listOf(str("x")), txn) }
            list.push(listOf(str("outside")))
        }
        assertEquals(listOf("editor"), events.map { it.originString })
        assertEquals(link, events.single().target)
        assertEquals(listOf(true, true, true), locked)
        assertEquals(listOf(num(1), out("x"), num(2)), link.unquote())

        // A link to a map hears the map's own changes deeply.
        val meta = notes.setMap("meta")
        val toMeta = notes.setWeakLink("toMeta", notes.link("meta")!!)
        val deep = Collections.synchronizedList(mutableListOf<YDeepEvent>())
        toMeta.observeDeep { deep += it }.use { meta.set("color", str("red")) }
        assertEquals(meta, (deep.single().events.single() as YEvent.MapChange).target)
    }
}
