import { defineRule, RuleType, RuleResultItem } from "../interface";
import { isType } from "../../walker/css";

const resultNoPseudoClass: RuleResultItem = {
  description: "使用了伪类",
  advice: "改为 CSS 类",
};

const resultNoPseudoElement: RuleResultItem = {
  description: "使用了伪元素",
  advice: "改为真实 DOM 元素",
};

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
          },
        });
      }
    },
  });
});
