import com.planerist.ykt.*
import kotlin.test.Test
import kotlin.test.assertEquals

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

        val xmlStr = xml.toString(null)
        assertEquals("<p>hello</p>world", xmlStr)
    }
}