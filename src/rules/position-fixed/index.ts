import {
  defineRule,
  RuleType,
  createResultItem,
  RuleLevel,
} from "../interface";
import { isType } from "src/walker/css";
import * as CssTree from "css-tree";

const result = createResultItem({
  name: "position-fixed",
  description: "position: fixed 未指定元素偏移",
  advice: "position-fixed 需要指定 top/bottom left/right。",
  level: RuleLevel.Error,
});

const verboseResult = createResultItem({
  name: "position fixed",
  description: "skyline 暂不支持 stacking context",
  advice: "请自行确认 fixed 节点的 z-index 层级是否符合预期",
  level: RuleLevel.Info,
});

export default defineRule(
  { name: "position-fixed", type: RuleType.WXSS },
  (ctx) => {
    ctx.lifetimes({
      onVisit: (node, visitCtx) => {
        if (
          isType(node, "Declaration") &&
          node.property === "position" &&
          isType(node.value, "Value") &&
          node.value.children.some(
            (val) => isType(val, "Identifier") && val.name === "fixed"
          )
        ) {
          const loc = node.loc!;
          ctx.addResult({
            ...verboseResult,
            loc: {
              startLn: loc.start.line,
              endLn: loc.end.line,
              startCol: loc.start.column,
              endCol: loc.end.column,
              path: ctx.env.path,
            },
          });

          const hasLOrR = CssTree.find(visitCtx.block!, (node, item, list) => {
            return (
              isType(node, "Declaration") &&
              (node.property === "left" || node.property === "right")
            );
          });
          const hasTOrB = CssTree.find(visitCtx.block!, (node, item, list) => {
            return (
              isType(node, "Declaration") &&
              (node.property === "top" || node.property === "bottom")
            );
          });
          if (!hasLOrR || !hasTOrB) {
            ctx.addResult({
              ...result,
              loc: {
                startLn: loc.start.line,
                endLn: loc.end.line,
                startCol: loc.start.column,
                endCol: loc.end.column,
                path: ctx.env.path,
              },
            });
          }
        }
      },
    });
  }
);
