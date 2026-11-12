import com.planerist.ykt.*
import org.junit.jupiter.api.assertTimeoutPreemptively
import java.time.Duration
import java.util.Collections
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * Root types by name: the same name gives the same type; while the document is locked (an open
 * transaction, an observer's callback) the getters throw instead of waiting. Every case runs under
 * a timeout, so a regression to a waiting lock fails instead of hanging the suite.
 */
class RootTypeTest {
    private val timeout = Duration.ofSeconds(10)

    /** What each getter did: "throws" for the document's lock error. */
    private fun getters(doc: YDoc): List<String> {
        fun check(name: String, body: () -> AutoCloseable): String =
            try {
                body().close()
                "$name works"
            } catch (e: Exception.AnotherTx) {
                "$name throws"
            }
        return listOf(
            check("getText") { doc.getText("t") },
            check("getXmlFragment") { doc.getXmlFragment("x") },
            check("getMap") { doc.getMap("m") },
            check("getArray") { doc.getArray("a") },
        )
    }

    private val allThrow = listOf("getText throws", "getXmlFragment throws", "getMap throws", "getArray throws")
    private val allWork = listOf("getText works", "getXmlFragment works", "getMap works", "getArray works")

    @Test
    fun sameNameSameText() = assertTimeoutPreemptively(timeout) {
        val doc = YDoc()
        doc.getText("t").insert(0u, "hello")
        assertEquals("hello", doc.getText("t").toText())
        assertEquals(0u, doc.getXmlFragment("x").length())
    }

    @Test
    fun insideATransactionTheyThrow() = assertTimeoutPreemptively(timeout) {
        val doc = YDoc()
        doc.transact { assertEquals(allThrow, getters(doc)) }
        assertEquals(allWork, getters(doc))
    }

    @Test
    fun insideATextObserverTheyThrow() = assertTimeoutPreemptively(timeout) {
        val doc = YDoc()
        val text = doc.getText("t")
        val results = Collections.synchronizedList(mutableListOf<List<String>>())
        text.observe { results += getters(doc) }.use { text.insert(0u, "a") }
        assertEquals(listOf(allThrow), results)
    }

    @Test
    fun insideAnUpdateObserverOfARemoteUpdateTheyThrow() = assertTimeoutPreemptively(timeout) {
        val a = YDoc(YDocOptions(1u))
        a.getText("t").insert(0u, "hi")
        val b = YDoc(YDocOptions(2u))
        val results = Collections.synchronizedList(mutableListOf<List<String>>())
        b.observeUpdateV2 { results += getters(b) }.use {
            applyUpdateV2(b, encodeStateAsUpdateV2(a).toDirectByteBuffer())
        }
        assertEquals(listOf(allThrow), results)
        assertEquals("hi", b.getText("t").toText())
    }
}
