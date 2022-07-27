import { hasChildren } from "domhandler";
import { getLocationByNode } from "src/utils/dom-ast";
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
  level: RuleLevel.Verbose,
});
const resultScrollViewXY = createResultItem({
  subname: "scroll-view-x-y",
  description: `scroll-view 暂不支持水平垂直方向同时滚动`,
  advice: `skyline 后续版本会支持`,
  level: RuleLevel.Info,
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
        const { start, end, path } = getLocationByNode(node);
        ctx.addResult({
          ...resultScrollViewImproperType,
          loc: {
            startIndex: start!,
            endIndex: end!,
            path,
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
        DomUtils.getAttributeValue(node, "scroll-x") === "true" &&
        DomUtils.getAttributeValue(node, "scroll-x") === "true"
      ) {
        const { start, end, path } = getLocationByNode(node);
        ctx.addResult({
          ...resultScrollViewXY,
          loc: {
            startIndex: start!,
            endIndex: end!,
            path,
          },
        });
      }
      if (hasChildren(node)) {
        const trimedChildren = node.childNodes.filter((child) => {
          if (isType(child, "Tag")) return true;
          if (isType(child, "Text") && child.data.trim() !== "") return true;
          return false;
        });

        if (
          trimedChildren.length === 1 &&
          isType(trimedChildren[0], "Tag") &&
          !Reflect.has(trimedChildren[0].attribs, "wx:for")
        ) {
          const { start, end, path } = getLocationByNode(node);
          ctx.addResult({
            ...resultScrollViewOptimize,
            loc: {
              startIndex: start!,
              endIndex: end!,
              path,
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
