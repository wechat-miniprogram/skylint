import { Selector, SelectorList } from "css-tree";
import { isType } from "src/walker/css";

export const formatSelector = (selector: Selector) => {
  let str = "";
  selector.children.forEach((node) => {
    if (isType(node, "IdSelector")) {
      str += `#${node.name}`;
    } else if (isType(node, "TypeSelector")) {
      str += node.name;
    } else if (isType(node, "ClassSelector")) {
      str += `.${node.name}`;
    } else if (isType(node, "Combinator")) {
      str += node.name;
    } else if (isType(node, "AttributeSelector")) {
      let tmp = node.name.name;
      if (node.matcher) tmp += node.matcher;
      if (node.value) tmp += isType(node.value, "String") ? `"${node.value.value}"` : node.value.name;
      if (node.flags) tmp += ` ${node.flags}`;
      str += `[${tmp}]`;
    } else if (isType(node, "PseudoClassSelector")) {
      str += `:${node.name}`;
    } else if (isType(node, "PseudoElementSelector")) {
      str += `::${node.name}`;
    }
  });
  return str;
};

export const formatSelectorList = (selectorList: SelectorList) => {
  return selectorList.children
    .toArray()
    .flatMap((selector) => {
      if (!isType(selector, "Selector")) return [];
      return formatSelector(selector);
    })
    .join(", ");
};
