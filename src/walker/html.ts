import { Walker } from "./interface";
import type { defaultTreeAdapter } from "parse5";
export type Node = Parameters<typeof defaultTreeAdapter["getNodeSourceCodeLocation"]>[0];

export const walk: Walker<Node> = (node, callback) => {
  if (callback(node) === false) {
    return false;
  } else {
    if ("childNodes" in node) {
      for (const childNode of node.childNodes ?? []) {
        const ret = walk(childNode, callback);
        if (ret === false) return ret;
      }
    }
    return true;
  }
};
