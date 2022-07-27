import { defineRule, RuleType, RuleResultItem } from "../interface";

let scrollViewCount = 0;

const resultScrollViewNotFound: RuleResultItem = {
  description: "未找到 scroll-view",
  advice: "使用 scroll-view 改善性能",
};

const resultScrollViewImproperType: RuleResultItem = {
  description: "scroll-view 的 type 不为 list",
};

export default defineRule({ name: "scroll-view", type: RuleType.WXML }, (ctx) => {
  ctx.lifetimes({
    before: () => {
      scrollViewCount = 0;
    },
    onVisit: (node) => {
      if (node.nodeName !== "scroll-view") return;
      scrollViewCount++;
      for (const { name, value } of node.attrs) {
        if (!(name === "type" && value === "list")) {
          const loc = node.sourceCodeLocation!;
          ctx.addResult({
            ...resultScrollViewImproperType,
            loc: {
              startLn: loc.startLine,
              endLn: loc.endLine,
              startCol: loc.startCol,
              endCol: loc.endCol,
            },
          });
        }
      }
    },
    after: () => {
      if (scrollViewCount === 0) ctx.addResult(resultScrollViewNotFound);
    },
  });
});
