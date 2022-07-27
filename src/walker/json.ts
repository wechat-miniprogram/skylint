import { DefaultWalkerContext, Walker } from "./interface";
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

export type JSONWalker = Walker<Node, DefaultWalkerContext<Node>>

export const walk: JSONWalker = (node, callback, ctx = { parent: node }) => {
  if (!ctx) ctx = { parent: node };
  if (callback(node, ctx) === false) {
    return false;
  } else {
    const newCtx = { parent: node };
    if ("children" in node) {
      for (const childNode of node.children ?? []) {
        if (walk(childNode, callback, { ...newCtx }) === false) return false;
      }
    } else if (isType(node, "Property")) {
      if (walk(node.key, callback, { ...newCtx }) === false) return false;
      if (walk(node.value, callback, { ...newCtx }) === false) return false;
    }
    return true;
  }
};
