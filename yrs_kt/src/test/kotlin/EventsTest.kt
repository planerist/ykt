import com.planerist.ykt.*
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Deep observers with targets and paths, XML observers, and the document's after-transaction,
 * subdocument and destroy events; unsubscribing inside a callback; remote updates.
 */
class EventsTest {
    private fun <T> synchronizedList(): MutableList<T> = Collections.synchronizedList(mutableListOf())
    private fun key(k: String) = YPathSegment.Key(k)
    private fun index(i: Int) = YPathSegment.Index(i.toUInt())
    private fun paths(event: YDeepEvent): List<List<YPathSegment>> = event.events.map {
        when (it) {
            is YEvent.TextChange -> it.path
            is YEvent.MapChange -> it.path
            is YEvent.ArrayChange -> it.path
            is YEvent.XmlChange -> it.path
            is YEvent.XmlTextChange -> it.path
            is YEvent.WeakLinkChange -> it.path
        }
    }

    @Test
    fun mapDeepObserverPathsAndTargets() {
        val doc = YDoc()
        val root = doc.getMap("root")
        val list = root.setArray("list")
        val item = list.insertMap(0u)
        val title = item.setText("title")
        val events = synchronizedList<YDeepEvent>()
        root.observeDeep { events += it }.use {
            root.set("plain", YValue.Number(1.0))
            title.insert(0u, "hi")
            doc.transact("editor") { txn ->
                item.set("done", YValue.Bool(true), txn)
                list.push(listOf(YValue.String("x")), txn)
            }
        }

        assertEquals(
            listOf(
                listOf(listOf()),
                listOf(listOf(key("list"), index(0), key("title"))),
                listOf(listOf(key("list")), listOf(key("list"), index(0))),
            ),
            events.map(::paths),
        )
        val text = events[1].events[0] as YEvent.TextChange
        assertEquals(title, text.target)
        assertEquals(listOf(YDelta.YInsert(YValue.String("hi"), null)), text.delta)
        assertEquals("editor", events[2].originString)
    }

    @Test
    fun textDeepObserverHearsAnEmbeddedMap() {
        val text = YDoc().getText("t")
        text.insert(0u, "ab")
        val table = text.insertEmbedMap(1u)
        val events = synchronizedList<YDeepEvent>()
        text.observeDeep { events += it }.use { table.set("rows", YValue.Number(2.0)) }
        val map = events.single().events.single() as YEvent.MapChange
        assertEquals(table, map.target)
        assertEquals(listOf(index(1)), map.path)
        assertEquals(mapOf("rows" to YEntryChange.Inserted(YOutput.Value(YValue.Number(2.0)))), map.keys)
    }

    @Test
    fun remoteChangesReachADeepObserver() {
        val a = YDoc(YDocOptions(1u))
        val item = a.getArray("list").insertMap(0u, mapOf("k" to YValue.Number(1.0)))
        val b = YDoc(YDocOptions(2u))
        applyUpdateV2(b, encodeStateAsUpdateV2(a).toDirectByteBuffer())
        val events = synchronizedList<YDeepEvent>()
        b.getArray("list").observeDeep { events += it }.use {
            item.set("k", YValue.Number(2.0))
            applyUpdateV2(b, encodeStateAsUpdateV2(a).toDirectByteBuffer(), "server".toByteArray())
        }
        assertEquals(listOf("server"), events.map { it.originString })
        assertEquals(listOf(listOf(listOf(index(0)))), events.map(::paths))
    }

    @Test
    fun xmlObservers() {
        val fragment = YDoc().getXmlFragment("x")
        val fragmentEvents = synchronizedList<YXmlEvent>()
        val elementEvents = synchronizedList<YXmlEvent>()
        val p = YXmlElement("p")
        fragment.observe { fragmentEvents += it }.use {
            fragment.push(YXmlChild.Element(p))
            p.observe { elementEvents += it }.use {
                p.setAttribute("class", YValue.String("a"))
                p.setAttribute("class", YValue.String("b"))
            }
        }
        val inserted = (fragmentEvents.single().children.single() as YArrayDelta.Insert).v1.single() as YOutput.XmlElement
        assertEquals(p, inserted.element)
        assertEquals(
            listOf(
                mapOf("class" to YEntryChange.Inserted(YOutput.Value(YValue.String("a")))),
                mapOf("class" to YEntryChange.Updated(YOutput.Value(YValue.String("a")), YOutput.Value(YValue.String("b")))),
            ),
            elementEvents.map { it.attributes },
        )
    }

    @Test
    fun unsubscribeInsideADeepCallback() {
        val doc = YDoc()
        val root = doc.getMap("root")
        val calls = synchronizedList<Boolean>()
        lateinit var sub: YSubscription
        sub = root.observeDeep {
            calls += runCatching { doc.getMap("root") }.exceptionOrNull() is YException.Locked
            sub.unsubscribe()
        }
        root.set("a", YValue.Number(1.0))
        root.set("b", YValue.Number(2.0))
        assertEquals(listOf(true), calls)
        sub.close()
    }

    @Test
    fun documentEvents() {
        val doc = YDoc(YDocOptions(7u))
        // Made first: getting a root type the first time is a transaction too.
        val text = doc.getText("t")
        val root = doc.getMap("root")
        val after = synchronizedList<YAfterTransactionEvent>()
        val subdocs = synchronizedList<YSubdocsEvent>()
        val destroyed = synchronizedList<String>()
        val subdocsSub = doc.observeSubdocs { subdocs += it }
        // Subscribed last: registering a document observer is a transaction of its own.
        val afterSub = doc.observeAfterTransaction { after += it }
        text.insert(0u, "hello")
        doc.transact("empty") { }
        val page = root.setDoc("page", YDoc())
        page.observeDestroy { destroyed += it.guid }.use { page.destroyDoc() }

        assertEquals(listOf(null, "empty"), after.take(2).map { it.originString })
        assertEquals(5u, after[0].afterState[7uL])
        assertEquals(listOf(page.guid()), subdocs.first().added.map { it.guid() })
        assertEquals(listOf(page.guid()), subdocs.last().removed.map { it.guid() })
        assertEquals(listOf(page.guid()), destroyed)
        assertTrue(afterSub.unsubscribe())
        subdocsSub.close()
    }
}
