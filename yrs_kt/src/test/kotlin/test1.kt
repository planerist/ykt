import com.planerist.ykt.DocOptions
import com.planerist.ykt.YDoc
import java.util.*
import kotlin.test.Test
import kotlin.test.assertEquals

class test1 {
    @Test
    fun Test() {
        val doc = YDoc(
            options = DocOptions(
                clientId = 0u,
                guid = UUID.randomUUID().toString(),
                collectionId = null,
                gc = null,
                autoLoad = null,
                shouldLoad = null
            )
        )

        val txt = doc.getText("name")

        var tx = doc.transaction(origin = null)
        txt.insert(0u, "hello word", "{ \"bold\": true }", tx)
        tx.commit()

        val txtAfter = doc.getText("name")

        tx = doc.transaction(origin = null)
        assertEquals("hello word", txtAfter.toString(tx))
        assertEquals("hello word", txt.toString(tx))
        tx.commit()
    }
}