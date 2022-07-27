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
      if (node.nodeName !== "scroll-view") return;
      scrollViewCount++;
      let hasTypeList = false;
      let typeAttr = node.attrs.find((attr) => {
        const { name, value } = attr;
        if (name === "type") {
          if (value === "list") hasTypeList = true;
          return true;
        }
      });

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
            start: node.sourceCodeLocation!.startTag!.endOffset - 1,
            end: node.sourceCodeLocation!.startTag!.endOffset - 1,
            // start: node.sourceCodeLocation!.startTag!.startOffset,
            // end: node.sourceCodeLocation!.startTag!.endOffset,
          },
        });
        ctx.addASTPatch(() => {
          if (typeAttr) {
            typeAttr.value = "";
          } else {
            node.attrs.push({
              name: "type",
              value: "list",
            });
          }
        });
      }
    },
    after: () => {
      if (scrollViewCount === 0) ctx.addResult(resultScrollViewNotFound);
    },
  });
});
