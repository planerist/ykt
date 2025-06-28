import com.planerist.ykt.*
import com.planerist.ykt.YDelta.YInsert
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class YXmlTest {
    @Test
    fun TestSnapshotState() {
        val d1 = YDoc(YDocOptions(1u, gc = false))

        val xml: YXmlFragment = d1.getXmlFragment("xml")
        d1.transaction().use { txn ->
            xml.push(
                YXmlChild.Element(
                    YXmlElement(
                        "p", emptyMap(),
                        listOf(YXmlChild.Text(YXmlText("hello", emptyMap())))
                    )
                ),
                txn
            )

            xml.push(YXmlChild.Text(YXmlText("world", emptyMap())), txn)
        }

        val xmlStr = xml.toText(null)
        assertEquals("<p>hello</p>world", xmlStr)
    }


    @Test
    fun TestAttributes() {
        val doc = YDoc(YDocOptions(1u, gc = false))
        val root = doc.getXmlFragment("test")
        val xml = YXmlElement("div", emptyMap(), emptyList())

        root.push(YXmlChild.Element(xml))

        var actual: Map<String, String?> = doc.transaction().use { txn ->
            // Test setting and getting attributes
            xml.setAttribute("key1", "value1", txn)
            xml.setAttribute("key2", "value2", txn)

            // Get all attributes and convert them to a map
            xml.attributes(txn)
        }

        assertEquals(
            mapOf(
                "key1" to "value1",
                "key2" to "value2"
            ), actual
        )

        // Test removing attribute
        actual = doc.transaction().use { txn ->
            xml.removeAttribute("key1", txn)

            mapOf(
                "key1" to xml.getAttribute("key1", txn),
                "key2" to xml.getAttribute("key2", txn)
            )
        }

        assertEquals(
            mapOf(
                "key1" to null,
                "key2" to "value2"
            ), actual
        )
    }

    @Test
    fun TestXmlTextEmbed() {
        val doc = YDoc(YDocOptions(1u, gc = false))
        val xml = doc.getXmlFragment("test")

        doc.transaction().use { txn ->
            val text = createText("some text", mapOf("attr1" to "v1"))
            // Test setting and getting attributes
            xml.push(text, txn)
            text.v1.insertEmbed(
                text.v1.length(txn), createText("bold", mapOf("bold" to "true")),
                null, txn
            )
        }

        val xmlStr = xml.toText(null)
        assertEquals(1u, xml.length())
        val firstChild = xml.firstChild() as YXmlChild.Text
        assertEquals("some text", firstChild.toText())
        assertEquals(
            listOf(
                YInsert(YValue.String("some text"), null),
                YInsert(YValue.String("bold"), mapOf("bold" to YValue.String("true"))),
            ), firstChild.v1.toDelta()
        )
        assertEquals(
            xmlStr, "some text"
        )
    }

    @Test
    fun TestSiblings() {
        val d1 = YDoc(YDocOptions(1u, gc = false))
        val root = d1.getXmlFragment("test")

        // Create and insert elements in a transaction
        val first = d1.transaction().use { txn ->
            val p = createElement(
                "p",
                emptyMap(),
                listOf(YXmlChild.Text(YXmlText("hello")))
            )
            assertTrue(p.v1.prelim())

            root.push(p, txn)
            root.push(createText("world"), txn)

            p
        }

        assertFalse(first.v1.prelim())
        // Test prevSibling
        assertEquals(null, first.v1.prevSibling())

        // Test nextSibling
        val second = (first.v1.nextSibling() as YXmlChild.Text).v1
        assertEquals("world", second.toText(null))
        assertEquals(null, second.nextSibling())

        // Compare prevSibling with first element
        assertEquals(first.toText(null), second.prevSibling()?.toText(null))
    }
}