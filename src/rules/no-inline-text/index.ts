import { hasChildren } from "domhandler";
import { DomUtils, isType } from "src/walker/html";
import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";

let scrollViewCount = 0;

const result = createResultItem({
  subname: "resultNoInlineText",
  description: "多段文本内联只能使用 text 组件包裹",
  advice: "目前不支持 inline ，需通过 text 组件实现，如 <view> foo <text>bar</text> </view> 要改为 <text> foo <text>bar</text> </text>",
  level: RuleLevel.Error,
});

export default defineRule({ name: "no-inline-text", type: RuleType.WXML }, (ctx) => {
  ctx.lifetimes({
    before: () => {
      scrollViewCount = 0;
    },
    onVisit: (node) => {
      if (!isType(node, "Tag") || node.name === "text") return;
      if (!hasChildren(node)) return;
      let isPrevText = false;
      for (const child of node.childNodes) {
        const isText =
          (isType(child, "Text") && child.data.trim() !== "") || (isType(child, "Tag") && child.name === "text");
        if (isText && isPrevText) {
          ctx.addResult({
            ...result,
            loc: {
              startIndex: node.startIndex!,
              endIndex: node.endIndex!,
            },
          });
          break;
        }
        isPrevText = isText;
      }
    },
  });
});
