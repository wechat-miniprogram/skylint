import { DomUtils, isType } from "src/walker/html";
import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";

let scrollViewCount = 0;

const resultScrollViewNotFound = createResultItem({
  subname: "resultScrollViewNotFound",
  description: "未找到 scroll-view",
  advice: "使用 scroll-view 实现滚动",
  level: RuleLevel.Warn,
});

const resultScrollViewImproperType = createResultItem({
  subname: "resultScrollViewImproperType",
  description: `scroll-view 的 type 不为 "list"`,
  advice: `设置 type 为 "list"`,
  fixable: true,
  level: RuleLevel.Error,
});

export default defineRule({ name: "scroll-view", type: RuleType.WXML }, (ctx) => {
  ctx.lifetimes({
    before: () => {
      scrollViewCount = 0;
    },
    onVisit: (node) => {
      if (!isType(node, "Tag") || node.name !== "scroll-view") return;
      scrollViewCount++;
      let hasTypeList = DomUtils.getAttributeValue(node, "type") === "list";

      if (!hasTypeList) {
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
        ctx.addPatch({
          patchedStr: ` type="list"`,
          loc: {
            start: node.startIndex! - 1,
            end: node.startIndex! - 1,
          },
        });
      }
    },
    after: () => {
      if (scrollViewCount === 0) ctx.addResult(resultScrollViewNotFound);
    },
  });
});
