import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

const resultNoPseudoClass = createResultItem({
  subname: "resultNoPseudoClass",
  description: "不支持伪类",
  advice:
    "需要改为通过 JS 或 wxml 模板语法的添加额外的 class 实现。skyline 未来版本会考虑支持 :first-child 和 :last-child",
  level: RuleLevel.Error,
  withCodeFrame: true,
});

const resultNoPseudoElement = createResultItem({
  subname: "resultNoPseudoElement",
  description: "不支持伪元素",
  advice: `需要改为添加真实的 wxml 节点实现，若是实现 "1px" 1 物理像素的效果，可直接使用小数点，如 0.5px`,
  level: RuleLevel.Error,
  withCodeFrame: true,
});

export default defineRule({ name: "no-pseudo", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType(node, "PseudoClassSelector")) {
        const loc = node.loc!;
        ctx.addResult({
          ...resultNoPseudoClass,
          loc: {
            startLn: loc.start.line,
            endLn: loc.end.line,
            startCol: loc.start.column,
            endCol: loc.end.column,
            path: ctx.env.path,
          },
        });
      } else if (isType(node, "PseudoElementSelector")) {
        const loc = node.loc!;
        ctx.addResult({
          ...resultNoPseudoElement,
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
