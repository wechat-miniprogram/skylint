import chalk from "chalk";
import {
  Node,
  ChildNode,
  Document,
  ParentNode,
  isText,
  isComment,
  isTag,
  Element,
  Text,
  Comment,
  isCDATA,
  CDATA,
  isDocument,
  isDirective,
  ProcessingInstruction,
} from "domhandler";
import {} from "htmlparser2";

interface Transformer<T extends Node = Node> {
  (node: T): void;
}

interface Option {
  transform?: (node: Transformer<ChildNode>) => void;
  /** @default true */
  keepLocation?: boolean;
  /** @default false */
  attachFilename?: string | false;
}

const nodeFilenameMap = new WeakMap<Node, string>();

export const attachFilenameToNode = (node: Node, filename: string) => {
  nodeFilenameMap.set(node, filename);
};

export const getLocationByNode = (node: Node) => {
  const { startIndex: start, endIndex: end } = node;
  const path = nodeFilenameMap.get(node) ?? null;
  return { start, end, path };
};

/**
 * Clone a node, and optionally its children. Copied from domhandler.
 *
 * @param recursive Clone child nodes as well.
 * @returns A clone of the node.
 */
export function cloneNode<T extends Node>(node: T, recursive = false, transform?: Transformer<T>): T {
  let result: Node;

  if (isText(node)) {
    result = new Text(node.data);
  } else if (isComment(node)) {
    result = new Comment(node.data);
  } else if (isTag(node)) {
    // @ts-ignore
    const children = recursive ? cloneChildren(node.children, transform) : [];
    const clone = new Element(node.name, { ...node.attribs }, children);
    children.forEach((child) => (child.parent = clone));

    if (node.namespace != null) {
      clone.namespace = node.namespace;
    }
    if (node["x-attribsNamespace"]) {
      clone["x-attribsNamespace"] = { ...node["x-attribsNamespace"] };
    }
    if (node["x-attribsPrefix"]) {
      clone["x-attribsPrefix"] = { ...node["x-attribsPrefix"] };
    }

    result = clone;
  } else if (isCDATA(node)) {
    const children = recursive ? cloneChildren(node.children) : [];
    const clone = new CDATA(children);
    children.forEach((child) => (child.parent = clone));
    result = clone;
  } else if (isDocument(node)) {
    const children = recursive ? cloneChildren(node.children) : [];
    const clone = new Document(children);
    children.forEach((child) => (child.parent = clone));

    if (node["x-mode"]) {
      clone["x-mode"] = node["x-mode"];
    }

    result = clone;
  } else if (isDirective(node)) {
    const instruction = new ProcessingInstruction(node.name, node.data);

    if (node["x-name"] != null) {
      instruction["x-name"] = node["x-name"];
      instruction["x-publicId"] = node["x-publicId"];
      instruction["x-systemId"] = node["x-systemId"];
    }

    result = instruction;
  } else {
    throw new Error(`Not implemented yet: ${node.type}`);
  }

  result.startIndex = node.startIndex;
  result.endIndex = node.endIndex;

  if (node.sourceCodeLocation != null) {
    result.sourceCodeLocation = node.sourceCodeLocation;
  }

  transform?.(result as T);

  return result as T;
}

/**
 * Clone a list of child nodes.
 *
 * @param childs The child nodes to clone.
 * @returns A list of cloned child nodes.
 */
function cloneChildren(childs: ChildNode[], transform?: Transformer<ChildNode>): ChildNode[] {
  const children = childs.map((child) => cloneNode(child, true, transform));

  for (let i = 1; i < children.length; i++) {
    children[i].prev = children[i - 1];
    children[i - 1].next = children[i];
  }

  return children;
}

export const replaceChildWithChildren = (child: ChildNode, children: ChildNode[], option: Option = {}) => {
  const parent = child.parentNode;
  if (!parent) return false;
  const { transform, keepLocation = true, attachFilename = false } = option;
  parent.children = parent.children.flatMap((childNode) => {
    if (childNode === child) {
      const newChildren = cloneChildren(children, (newChild) => {
        if (keepLocation) {
          if (attachFilename) attachFilenameToNode(newChild, attachFilename);
        } else {
          newChild.startIndex = newChild.endIndex = null;
        }
        // @ts-ignore
        transform?.(newChild);
      });
      newChildren.forEach((child) => (child.parent = parent));
      const firstChild = newChildren[0];
      const lastChild = newChildren[newChildren.length - 1];
      if (firstChild) {
        if (child.prev) child.prev.next = firstChild;
        firstChild.prev = child.prev;
      }
      if (lastChild) {
        if (child.next) child.next.prev = lastChild;
        lastChild.next = child.next;
      }
      return newChildren;
    }
    return childNode;
  });
  return true;
};

export const naivePrint = (ast: Node) => {
  console.log(
    JSON.stringify(
      ast,
      (key, value) => {
        if (["parentNode", "parent", "next", "prev", "sourceCodeLocation"].includes(key)) return undefined;
        if (key === "tagName") return chalk.green(value);
        return value;
      },
      2
    )
  );
};
