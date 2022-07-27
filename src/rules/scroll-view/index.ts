import { hasChildren } from "domhandler";
import { DomUtils, isType } from "src/walker/html";
import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";

let scrollViewCount = 0;

const resultScrollViewNotFound = createResultItem({
  subname: "scroll-view-not-found",
  description: "当前页面未使用 scroll-view 组件",
  advice: "skyline 不支持页面全局滚动，若页面超过一屏，需要使用 scroll-view 组件实现滚动",
  level: RuleLevel.Warn,
});

const resultScrollViewImproperType = createResultItem({
  subname: "scroll-view-type",
  description: `scroll-view 未显式指定 type 类型`,
  advice: `当前 scroll-view 只支持 type=list 且需显式指定，详见文档（链接待定）`,
  fixable: true,
  level: RuleLevel.Error,
});

const resultScrollViewOptimize = createResultItem({
  subname: "scroll-view-optimize",
  description: `未能充分利用 scroll-view 按需渲染的机制`,
  advice: `scroll-view 会根据直接子节点是否在屏来按需渲染，若只有一个直接子节点则性能会退化，如 <scroll-view type=list scroll-y> <view wx:for=""/> </scroll-view>`,
  fixable: true,
  level: RuleLevel.Verbose,
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
        ctx.addResult({
          ...resultScrollViewImproperType,
          loc: {
            startIndex: node.startIndex!,
            endIndex: node.endIndex!,
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
      if (
        hasChildren(node) &&
        node.childNodes.length === 1 &&
        isType(node.childNodes[0], "Tag") &&
        !Reflect.has(node.attribs, "wx:for")
      ) {
        ctx.addResult({
          ...resultScrollViewOptimize,
          loc: {
            startIndex: node.startIndex!,
            endIndex: node.endIndex!,
          },
        });
      }
    },
    after: () => {
      if (scrollViewCount === 0) ctx.addResult(resultScrollViewNotFound);
    },
  });
});
