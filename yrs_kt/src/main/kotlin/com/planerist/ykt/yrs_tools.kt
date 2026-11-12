package com.planerist.ykt

import java.nio.ByteBuffer

inline fun <R> YDoc.transact(origin: String? = null, block: (txn : YTransaction) -> R): R {
    return this.transaction(origin).use { txn ->
        block(txn)
    }
}

fun createXmlElement(
    name: String,
    attributes: Map<String, YValue>? = null,
    children: List<YXmlChild>? = null
): YXmlChild.Element {
    return YXmlChild.Element(YXmlElement(name, attributes, children))
}

fun createXmlText(
    text: String,
    attributes: Map<String, YValue>? = null,
): YXmlChild.Text {
    return YXmlChild.Text(YXmlText(text, attributes))
}

fun createXmlFragment(
    children: List<YXmlChild>
): YXmlChild.Fragment {
    return YXmlChild.Fragment(YXmlFragment(children))
}

fun YXmlChild.toText(txn: YTransaction? = null): String {
    return when (this) {
        is YXmlChild.Element -> this.v1.toText(txn)
        is YXmlChild.Fragment -> this.v1.toText(txn)
        is YXmlChild.Text -> this.v1.toText(txn)
    }
}

fun stringYValue(value: String) =
    YValue.String(
        value
    )

fun booleanYValue(value: Boolean) =
    YValue.Bool(
        value
    )

fun longYValue(value: Long) =
    YValue.BigInt(
        value
    )


fun numberYValue(value: Double) =
    YValue.Number(
        value
    )

// zero-copy `&[u8]` arguments (applyUpdate, decodeSnapshotV1, ...) accept only direct buffers
fun ByteArray.toDirectByteBuffer(): ByteBuffer =
    ByteBuffer.allocateDirect(size).put(this).flip()

// Observers with lambdas. The callbacks run synchronously on the thread that commits the
// transaction, while the document is still locked: they must not read or write the document (that
// throws); everything they need is in the event. Keep the returned YSubscription: `close()` (or
// `unsubscribe()`) stops them; it is AutoCloseable, so `use { }` works.

/** Yjs's `ytext.observe`: the text's change as a delta, after every committed transaction that changed it. */
fun YText.observe(onChange: (YTextEvent) -> Unit): YSubscription =
    observe(object : YTextObserver {
        override fun onChange(event: YTextEvent) = onChange(event)
    })

/** Yjs's `doc.on('update')`: every committed transaction's changes as one update (lib0 v1). */
fun YDoc.observeUpdateV1(onUpdate: (YUpdateEvent) -> Unit): YSubscription =
    observeUpdateV1(object : YUpdateObserver {
        override fun onUpdate(event: YUpdateEvent) = onUpdate(event)
    })

/** Yjs's `doc.on('updateV2')`: every committed transaction's changes as one update (lib0 v2). */
fun YDoc.observeUpdateV2(onUpdate: (YUpdateEvent) -> Unit): YSubscription =
    observeUpdateV2(object : YUpdateObserver {
        override fun onUpdate(event: YUpdateEvent) = onUpdate(event)
    })

/** The state vectors before and after every committed transaction (an empty one too). */
fun YDoc.observeTransactionCleanup(onCleanup: (YTransactionCleanupEvent) -> Unit): YSubscription =
    observeTransactionCleanup(object : YTransactionCleanupObserver {
        override fun onCleanup(event: YTransactionCleanupEvent) = onCleanup(event)
    })

/** The transaction's origin as text (`transaction(origin)` takes a string). */
val YTextEvent.originString: String? get() = origin?.decodeToString()
val YUpdateEvent.originString: String? get() = origin?.decodeToString()
val YTransactionCleanupEvent.originString: String? get() = origin?.decodeToString()
