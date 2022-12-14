import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType, Node } from "src/walker/html";
import { hasChildren } from "domhandler";

const result = createResultItem({
  name: "no-svg-style-tag",
  description: "不支持 svg 组件内使用 style 标签",
  advice: "在 wxss 文件内书写样式规则",
  level: RuleLevel.Error,
  withCodeFrame: true,
});

export default defineRule({ name: "no-svg-style-tag", type: RuleType.WXML }, (ctx) => {
  const dfs = (node: Node, ruleCtx: typeof ctx) => {
    if ((isType(node, "Tag") && node.name === "style") || isType(node, "Style")) {
      ruleCtx.addResult({
        ...result,
        loc: {
          startIndex: node.startIndex!,
          endIndex: node.endIndex!,
          path: ctx.env.path,
        },
      });
    }
    if (hasChildren(node)) {
      for (const childNode of node.childNodes) {
        dfs(childNode, ruleCtx);
      }
    }
  };
  ctx.lifetimes({
    onVisit: (node, walkCtx) => {
      if (isType(node, "Tag") && node.name === "svg" && hasChildren(node)) {
        node.childNodes.forEach((childNode) => dfs(childNode, ctx));
      }
    },
  });
});
