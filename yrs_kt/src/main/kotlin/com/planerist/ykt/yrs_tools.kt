package com.planerist.ykt

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