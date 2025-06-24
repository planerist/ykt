import com.planerist.ykt.*
import com.planerist.ykt.YDelta.YInsert
import org.junit.jupiter.api.assertThrows
import kotlin.test.Test
import kotlin.test.assertEquals

class YXmlTest {
    @Test
    fun TestSnapshotState() {
        val d1 = YDoc(YDocOptions(1u, gc = false))

        val text = d1.getText("xml")
        d1.transaction().use {
                        root.push(new Y.YXmlElement('p', {}, [
                new Y.YXmlText('hello')
            ]), txn)
            root.push(new Y.YXmlText('world'), txn)
        }

        const s = root.toString()
        t.compareStrings(s, '<p>hello</p>world')

    }
}