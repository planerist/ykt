import com.planerist.ykt.*
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals

class YDocTest {
    @Test
    fun TestSnapshotState() {
        val d1 = YDoc(YDocOptions(1u, gc = false))

        val text = d1.getText("text")
        text.insert(0u, "hello")

        val prev = snapshot(d1)
        text.insert(5u, " world")

        val state = encodeStateFromSnapshotV1(d1, prev)
        assertContentEquals(
            ubyteArrayOf(
                1u,
                1u,
                1u,
                0u,
                4u,
                1u,
                4u,
                116u,
                101u,
                120u,
                116u,
                5u,
                104u,
                101u,
                108u,
                108u,
                111u,
                0u
            ), state.asUByteArray()
        )

        val d2 = YDoc(YDocOptions(2u))
        val txt2 = d2.getText("text")
        applyUpdate(d2, state)


        assertEquals(txt2.getText(), "hello")
    }
}