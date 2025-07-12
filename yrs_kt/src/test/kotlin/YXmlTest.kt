import com.planerist.ykt.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class YXmlTest {
    fun dumpAttrs(attributes: Map<String, Any?>): String {
        return attributes.entries.joinToString(", ") { (key, value) ->
            "\"$key\": '$value'}"
        } + ", "
    }

    fun dumpXmlText(xml: YXmlText): String {
        val result = StringBuilder("{ ")

        result.append("\"type\": \"text\" ")
        result.append(dumpAttrs(xml.attributes()))

        val delta = xml.toDelta()
        if (delta.isNotEmpty()) {
            result.append("\"children\": [\n")
            delta.forEachIndexed { index, d ->
                when (d) {
                    is YXmlDelta.YInsert -> {
                        when (d.v1) {
                            is YDeltaXmlChild.Text -> {
                                result.append(dumpXmlText(d.v1.v1).prependIndent())
                            }

                            is YDeltaXmlChild.Embed -> {
                                val value = (d.v1.v1 as YValue.String).v1
                                val attrs = d.v1.v2
                                result.append("<$value".prependIndent())
                                if(attrs !== null) {
                                    result.append(", " + dumpAttrs(attrs))
                                }
                                result.append(">")
                            }

                            else -> throw IllegalStateException("Unsupported element type ${d.v1::class.simpleName}")
                        }
                    }

                    else -> error("unsupported op")
                }

                if (index < delta.size - 1) {
                    result.append(",\n")
                }
            }
            result.append("\n]\n")
        }

        return result.append("}").toString()
    }

    fun dumpElement(elem: YXmlElement): String {
        val result = StringBuilder("{ ")

        result.append("\"elem\": '${elem.toText()}', " + dumpAttrs(elem.attributes()))

        var node = elem.firstChild()

        while (node !== null) {
            when (node) {
                is YXmlChild.Element -> result.append(dumpElement(node.v1))
                is YXmlChild.Text -> result.append(dumpXmlText(node.v1))
                is YXmlChild.Fragment -> TODO("Should never happen?")
            }

            node = when (node) {
                is YXmlChild.Element -> node.v1.nextSibling()
                is YXmlChild.Fragment -> TODO("Should never happen?")
                is YXmlChild.Text -> node.v1.nextSibling()
            }
        }

        return result.append(" }").toString()
    }


    fun dumpFragment(frag: YXmlFragment): String {
        val result = StringBuilder("[\n")

        var node = frag.firstChild()

        while (node !== null) {
            when (node) {
                is YXmlChild.Element -> result.append(dumpElement(node.v1).prependIndent())
                is YXmlChild.Text -> result.append(dumpXmlText(node.v1).prependIndent())
                is YXmlChild.Fragment -> TODO("Should never happen?")
            }

            node = when (node) {
                is YXmlChild.Element -> node.v1.nextSibling()
                is YXmlChild.Fragment -> TODO("Should never happen?")
                is YXmlChild.Text -> node.v1.nextSibling()
            }

            result.append(",\n")
        }

        return result.append("]\n").toString()
    }

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

        var actual: Map<String, YValue?> = doc.transaction().use { txn ->
            // Test setting and getting attributes
            xml.setAttribute("key1", stringYValue("value1"), txn)
            xml.setAttribute("key2", longYValue(42), txn)
            xml.setAttribute("key3", booleanYValue(false), txn)

            // Get all attributes and convert them to a map
            xml.attributes(txn)
        }

        assertEquals(
            mapOf(
                "key1" to stringYValue("value1"),
                "key2" to longYValue(42),
                "key3" to booleanYValue(false)
            ), actual
        )

        // Test removing attribute
        actual = doc.transaction().use { txn ->
            xml.removeAttribute("key1", txn)

            mapOf(
                "key1" to xml.getAttribute("key1", txn),
                "key2" to xml.getAttribute("key2", txn),
                "key3" to xml.getAttribute("key3", txn)
            )
        }

        assertEquals(
            mapOf(
                "key1" to null,
                "key2" to longYValue(42),
                "key3" to booleanYValue(false)
            ), actual
        )
    }

    @Test
    fun TestXmlTextEmbed() {
        val doc = YDoc(YDocOptions(1u, gc = false))
        val xml = doc.getXmlFragment("test")

        doc.transaction().use { txn ->
            val text = createXmlText("some text", mapOf("attr1" to stringYValue("attr_value")))
            // Test setting and getting attributes
            xml.push(text, txn)
            text.v1.insertEmbed(
                text.v1.length(txn), createXmlText("bold", mapOf("bold" to booleanYValue(true))),
                null, txn
            )

            text.v1.insert(text.v1.length(txn),"text", mapOf("attr2" to numberYValue(42.0)), txn)
        }

        assertEquals(1u, xml.length())
        val firstChild = xml.firstChild() as YXmlChild.Text
        assertEquals(
            """[
    { "type": "text" "attr1": 'String(v1=attr_value)'}, "children": [
        <some text>,
        { "type": "text" "bold": 'Bool(v1=true)'}, "children": [
            <bold>
        ]
        },
        <text, "attr2": 'Number(v1=42.0)'}, >
    ]
    },
]
""", dumpFragment(xml)
        )
    }

    @Test
    fun TestSiblings() {
        val d1 = YDoc(YDocOptions(1u, gc = false))
        val root = d1.getXmlFragment("test")

        // Create and insert elements in a transaction
        val first = d1.transaction().use { txn ->
            val p = createXmlElement(
                "p",
                emptyMap(),
                listOf(YXmlChild.Text(YXmlText("hello")))
            )
            assertTrue(p.v1.prelim())

            root.push(p, txn)
            root.push(createXmlText("world"), txn)

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

    @Test
    fun TestToDelta() {
        val doc = YDoc(YDocOptions(1u, gc = false))
        val xml = doc.getXmlFragment("test")

        doc.transaction().use { txn ->
            val text = createXmlText("some text", mapOf("attr1" to stringYValue("v1")))
            // Test setting and getting attributes
            xml.push(text, txn)

            val embed1 = createXmlText("bold", mapOf("bold" to booleanYValue(true)))
            text.v1.insertEmbed(
                text.v1.length(txn), embed1,
                null, txn
            )

            embed1.v1.insertEmbed(embed1.v1.length(txn), createXmlText("sub-bold"), null, txn)
        }

        assertEquals(
            """[
    { "type": "text" "attr1": 'String(v1=v1)'}, "children": [
        <some text>,
        { "type": "text" "bold": 'Bool(v1=true)'}, "children": [
            <bold>,
            { "type": "text" , "children": [
                <sub-bold>
            ]
            }
        ]
        }
    ]
    },
]
""", dumpFragment(xml)
        )
    }
}