package com.planerist.ykt

fun createElement(
    name: String,
    attributes: Map<String, String>? = null,
    children: List<YXmlChild>? = null
): YXmlChild.Element {
    return YXmlChild.Element(YXmlElement(name, attributes, children))
}

fun createText(
    text: String,
    attributes: Map<String, String>? = null,
): YXmlChild.Text {
    return YXmlChild.Text(YXmlText(text, attributes))
}

fun createFragment(
    children: List<YXmlChild>
): YXmlChild.Fragment {
    return YXmlChild.Fragment(YXmlFragment(children))
}

fun YXmlChild.toText(txn: YTransaction?) {
    when(this) {
        is YXmlChild.Element -> this.v1.toText(txn)
        is YXmlChild.Fragment -> this.v1.toText(txn)
        is YXmlChild.Text -> this.v1.toText(txn)
    }
}