import com.planerist.ykt.*
import org.junit.jupiter.api.assertThrows
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
        d1.transact { txn ->
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

        var actual: Map<String, YValue?> = doc.transact { txn ->
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
                "key2" to numberYValue(42.0),
                "key3" to booleanYValue(false)
            ), actual
        )

        // Test removing attribute
        actual = doc.transact { txn ->
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
                "key2" to numberYValue(42.0),
                "key3" to booleanYValue(false)
            ), actual
        )
    }

    @Test
    fun TestIntegerAttributes() {
        val doc = YDoc(YDocOptions(1u, gc = false))
        val root = doc.getXmlFragment("test")
        val xml = YXmlElement("div", emptyMap(), emptyList())

        root.push(YXmlChild.Element(xml))

        val maxSafe = (1L shl 53) - 1
        val bigInt = 1L shl 60
        val actual = doc.transact { txn ->
            xml.setAttribute("safe", longYValue(maxSafe), txn)
            xml.setAttribute("big", longYValue(bigInt), txn)
            xml.setAttribute("negBig", longYValue(-bigInt), txn)
            xml.attributes(txn)
        }

        assertEquals(
            mapOf(
                "safe" to numberYValue(maxSafe.toDouble()),
                "big" to longYValue(bigInt),
                "negBig" to longYValue(-bigInt)
            ), actual
        )
    }

    @Test
    fun TestXmlTextEmbed() {
        val doc = YDoc(YDocOptions(1u, gc = false))
        val xml = doc.getXmlFragment("test")

        doc.transact { txn ->
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
        val first = d1.transact { txn ->
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

        doc.transact { txn ->
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

    @Test
    fun applyDeltaToAnXmlText() {
        val a = YDoc(YDocOptions(1u))
        val text = createXmlText("hello world").v1
        a.getXmlFragment("x").push(YXmlChild.Text(text))
        text.applyDelta(
            listOf(
                YDelta.YRetain(5u, mapOf("bold" to YValue.Bool(true))),
                YDelta.YInsert(YValue.String(","), null),
                YDelta.YRetain(1u, null),
                YDelta.YDelete(5u),
                YDelta.YInsert(YValue.String("there"), mapOf("italic" to YValue.Bool(true))),
            )
        )
        assertEquals("<bold>hello</bold>, <italic>there</italic>", text.toText())

        val b = YDoc(YDocOptions(2u))
        applyUpdateV2(b, encodeStateAsUpdateV2(a).toDirectByteBuffer())
        val replica = (b.getXmlFragment("x").firstChild() as YXmlChild.Text).v1
        assertEquals(text.toDelta(), replica.toDelta())

        // A delete past the end throws before anything is applied; a preliminary text has nothing to apply it to.
        assertThrows<YException.OutOfBounds> { text.applyDelta(listOf(YDelta.YInsert(YValue.String("!"), null), YDelta.YDelete(20u))) }
        assertEquals("<bold>hello</bold>, <italic>there</italic>", text.toText())
        assertThrows<YException.NotIntegrated> { YXmlText("x").applyDelta(listOf(YDelta.YDelete(1u))) }

        // Inside its own callback, the document is locked.
        val locked = mutableListOf<Boolean>()
        text.observe { locked += runCatching { text.applyDelta(listOf(YDelta.YDelete(1u))) }.exceptionOrNull() is YException.Locked }
            .use { text.applyDelta(listOf(YDelta.YRetain(5u, mapOf("bold" to YValue.Null)))) }
        assertEquals(listOf(true), locked)
        assertEquals("hello, <italic>there</italic>", text.toText())
    }

    private fun describe(nodes: List<YXmlChild>): List<String> = nodes.map {
        when (it) {
            is YXmlChild.Element -> "<${it.v1.name()}>"
            is YXmlChild.Text -> it.v1.toText()
            is YXmlChild.Fragment -> "#fragment"
        }
    }

    @Test
    fun treeWalker() {
        val a = YDoc(YDocOptions(1u))
        val root = a.getXmlFragment("x")
        val b = YXmlElement("b", children = listOf(createXmlText("world")))
        val p = YXmlElement("p", children = listOf(createXmlText("Hello "), YXmlChild.Element(b)))
        assertEquals(listOf("Hello ", "<b>", "world"), describe(p.treeWalker()), "a new node's tree")
        root.push(YXmlChild.Element(p))
        root.push(createXmlText("again"))
        root.push(createXmlElement("hr"))

        val everything = listOf("<p>", "Hello ", "<b>", "world", "again", "<hr>")
        assertEquals(everything, describe(root.treeWalker()))
        assertEquals(listOf("Hello ", "<b>", "world"), describe(p.treeWalker()), "only what is inside")
        assertEquals(b, (root.treeWalker()[2] as YXmlChild.Element).v1, "a handle of the node itself")

        val replica = YDoc(YDocOptions(2u))
        applyUpdateV2(replica, encodeStateAsUpdateV2(a).toDirectByteBuffer())
        assertEquals(everything, describe(replica.getXmlFragment("x").treeWalker()))

        root.delete(0u, 1u)
        assertEquals(listOf("again", "<hr>"), describe(root.treeWalker()), "deleted nodes are left out")
        val locked = mutableListOf<Boolean>()
        root.observe { locked += runCatching { root.treeWalker() }.exceptionOrNull() is YException.Locked }
            .use { root.delete(0u, 2u) }
        assertEquals(listOf(true), locked)
        assertEquals(emptyList(), root.treeWalker())
    }
}
