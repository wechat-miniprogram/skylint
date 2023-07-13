import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

const supportedPseudoClass = new Set(["first-child", "last-child"]);
const resultNoPseudoClass = (name: string) =>
  createResultItem({
    name: "no-pseudo-class",
    description: `不支持的伪类 :${name}`,
    advice: "需要改为通过 JS 或 wxml 模板语法的添加额外的 class 实现。",
    level: RuleLevel.Error,
    withCodeFrame: true,
  });

const supportedPseudoElement = new Set(["before", "after"]);
const resultNoPseudoElement = (name: string) =>
  createResultItem({
    name: "no-pseudo-element",
    description: `不支持的伪元素 ::${name}`,
    advice: `需要改为添加真实的 wxml 节点实现，若是实现 "1px" 1 物理像素的效果，可直接使用小数点，如 0.5px`,
    level: RuleLevel.Error,
    withCodeFrame: true,
  });

export default defineRule({ name: "no-pseudo", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType(node, "PseudoClassSelector") && !supportedPseudoClass.has(node.name)) {
        const loc = node.loc!;
        ctx.addResult({
          ...resultNoPseudoClass(node.name),
          loc: {
            startLn: loc.start.line,
            endLn: loc.end.line,
            startCol: loc.start.column,
            endCol: loc.end.column,
            path: ctx.env.path,
          },
        });
      } else if (isType(node, "PseudoElementSelector") && !supportedPseudoElement.has(node.name)) {
        const loc = node.loc!;
        ctx.addResult({
          ...resultNoPseudoElement(node.name),
          loc: {
            startLn: loc.start.line,
            endLn: loc.end.line,
            startCol: loc.start.column,
            endCol: loc.end.column,
            path: ctx.env.path,
          },
        });
      }
    },
  });
});
