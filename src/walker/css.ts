import { Walker } from "./interface";
import { walk as walkCSS, type CssNode as Node, type WalkContext } from "css-tree";
import * as CssTree from "css-tree";

interface CSSNodeTypeMap {
  AnPlusB: CssTree.AnPlusB;
  Atrule: CssTree.Atrule;
  AtrulePrelude: CssTree.AtrulePrelude;
  AttributeSelector: CssTree.AttributeSelector;
  Block: CssTree.Block;
  Brackets: CssTree.Brackets;
  CDC: CssTree.CDC;
  CDO: CssTree.CDO;
  ClassSelector: CssTree.ClassSelector;
  Combinator: CssTree.Combinator;
  Comment: CssTree.Comment;
  Declaration: CssTree.Declaration;
  DeclarationList: CssTree.DeclarationList;
  Dimension: CssTree.Dimension;
  Function: CssTree.FunctionNode;
  Hash: CssTree.Hash;
  IdSelector: CssTree.IdSelector;
  Identifier: CssTree.Identifier;
  MediaFeature: CssTree.MediaFeature;
  MediaQuery: CssTree.MediaQuery;
  MediaQueryList: CssTree.MediaQueryList;
  Nth: CssTree.Nth;
  Number: CssTree.NumberNode;
  Operator: CssTree.Operator;
  Parentheses: CssTree.Parentheses;
  Percentage: CssTree.Percentage;
  PseudoClassSelector: CssTree.PseudoClassSelector;
  PseudoElementSelector: CssTree.PseudoElementSelector;
  Ratio: CssTree.Ratio;
  Raw: CssTree.Raw;
  Rule: CssTree.Rule;
  Selector: CssTree.Selector;
  SelectorList: CssTree.SelectorList;
  String: CssTree.StringNode;
  StyleSheet: CssTree.StyleSheet;
  TypeSelector: CssTree.TypeSelector;
  UnicodeRange: CssTree.UnicodeRange;
  Url: CssTree.Url;
  Value: CssTree.Value;
  WhiteSpace: CssTree.WhiteSpace;
}

export const isType = <T extends keyof CSSNodeTypeMap>(node: Node, type: T): node is CSSNodeTypeMap[T] => {
  return node.type === type;
};

export type CSSWalker = Walker<Node, WalkContext>;

export const walk: CSSWalker = (node, callback) => {
  return walkCSS(node, function (node) {
    const self = this;
    callback(node, self);
  });
};

export { type CssNode as Node } from "css-tree";
