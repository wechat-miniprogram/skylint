import { Walker } from "./interface";
import { ArrayNode, IdentifierNode, LiteralNode, ObjectNode, PropertyNode } from "json-to-ast";

interface JSONNodeMap {
  Array: ArrayNode;
  Identifier: IdentifierNode;
  Literal: LiteralNode;
  Object: ObjectNode;
  Property: PropertyNode;
}

export type Node = ArrayNode | IdentifierNode | LiteralNode | ObjectNode | PropertyNode;

export const isType = <T extends keyof JSONNodeMap>(node: Node, type: T): node is JSONNodeMap[T] => {
  return node.type === type;
};

export const walk: Walker<Node> = (node, callback) => {
  if (callback(node) === false) {
    return false;
  } else {
    if ("children" in node) {
      for (const childNode of node.children ?? []) {
        if (walk(childNode, callback) === false) return false;
      }
    } else if (isType(node, "Property")) {
      if (walk(node.key, callback) === false) return false;
      if (walk(node.value, callback) === false) return false;
    }
    return true;
  }
};
