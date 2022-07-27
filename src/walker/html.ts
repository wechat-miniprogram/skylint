import { DefaultWalkerContext, Walker } from "./interface";
export { DomUtils } from "htmlparser2";
import { Node, CDATA, Element, Document, Text, ProcessingInstruction, Comment, hasChildren } from "domhandler";
export { Node } from "domhandler";

export interface NodeTypeMap {
  Root: Document;
  Text: Text;
  Directive: ProcessingInstruction;
  Comment: Comment;
  Script: Element;
  Style: Element;
  Tag: Element;
  CDATA: CDATA;
  Doctype: ProcessingInstruction;
}

export const elementType = {
  /** Type for the root element of a document */
  Root: "root",
  /** Type for Text */
  Text: "text",
  /** Type for <? ... ?> */
  Directive: "directive",
  /** Type for <!-- ... --> */
  Comment: "comment",
  /** Type for <script> tags */
  Script: "script",
  /** Type for <style> tags */
  Style: "style",
  /** Type for Any tag */
  Tag: "tag",
  /** Type for <![CDATA[ ... ]]> */
  CDATA: "cdata",
  /** Type for <!doctype ...> */
  Doctype: "doctype",
};

export type HTMLWalker = Walker<Node, DefaultWalkerContext<Node>>;

export const isType = <T extends keyof typeof elementType>(node: Node, type: T): node is NodeTypeMap[T] => {
  return node.type === elementType[type];
};

export const walk: HTMLWalker = (node, callback, ctx = { parent: node }) => {
  if (!ctx) ctx = { parent: node };
  if (callback(node, ctx) === false) {
    return false;
  } else {
    if (hasChildren(node)) {
      for (const childNode of node.childNodes ?? []) {
        const ret = walk(childNode, callback, { ...ctx, parent: node });
        if (ret === false) return ret;
      }
    }
    return true;
  }
};
