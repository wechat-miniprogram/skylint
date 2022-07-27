import { hasChildren } from "domhandler";
import { DomUtils, isType } from "src/walker/html";
import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";

const result = createResultItem({
  subname: "",
  description: `未打开样式共享标记`,
  advice: `每一个列表项的样式是基本相同的，因此 skyline 实现了样式共享机制，可降低样式匹配的耗时，只需要列表项加个 list-item，如 <view wx:for="" list-item />`,
  level: RuleLevel.Verbose,
});

export default defineRule({ name: "mark-wx-for", type: RuleType.WXML }, (ctx) => {
  ctx.lifetimes({
    before: () => {},
    onVisit: (node) => {
      if (!isType(node, "Tag") || !Reflect.has(node.attribs, "wx:for")) return;
      if (node.name === "block" && hasChildren(node)) {
        for (const childNode of node.childNodes) {
          if (isType(childNode, "Tag") && !Reflect.has(childNode.attribs, "list-item")) {
            ctx.addResult({
              ...result,
              loc: {
                startIndex: childNode.startIndex!,
                endIndex: childNode.endIndex!,
                path: ctx.env.path,
              },
            });
          }
        }
      } else if (!Reflect.has(node.attribs, "list-item")) {
        ctx.addResult({
          ...result,
          loc: {
            startIndex: node.startIndex!,
            endIndex: node.endIndex!,
            path: ctx.env.path,
          },
        });
      }
    },
  });
});
