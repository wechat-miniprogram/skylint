import { hasChildren } from "domhandler";
import { selectAll } from "css-select";
import { getLocationByNode } from "src/utils/dom-ast";
import { formatSelectorList } from "src/utils/css-ast";
import { DomUtils, isType } from "src/walker/html";
import { isType as isTypeCSS } from "src/walker/css";
import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";

const resultScrollViewNotFound = createResultItem({
  name: "scroll-view-not-found",
  description: "当前页面未使用 scroll-view 组件",
  advice: "skyline 不支持页面全局滚动，若页面超过一屏，需要使用 scroll-view 组件实现滚动",
  level: RuleLevel.Warn,
});
const resultScrollViewImproperType = createResultItem({
  name: "scroll-view-type",
  description: `scroll-view 未显式指定 type 类型`,
  advice: `当前 scroll-view 只支持 type=list 且需显式指定，详见文档（链接待定）`,
  fixable: true,
  level: RuleLevel.Error,
});
const resultScrollViewOptimize = createResultItem({
  name: "scroll-view-optimize",
  description: `未能充分利用 scroll-view 按需渲染的机制`,
  advice: `scroll-view 会根据直接子节点是否在屏来按需渲染，若只有一个直接子节点则性能会退化，如 <scroll-view type=list scroll-y><view><view wx:for=""/></view></scroll-view>`,
  level: RuleLevel.Verbose,
});
const resultScrollViewXY = createResultItem({
  name: "scroll-view-x-y",
  description: `scroll-view 暂不支持水平垂直方向同时滚动`,
  advice: `skyline 后续版本会支持`,
  level: RuleLevel.Info,
});
const resultScrollMargin = createResultItem({
  name: "scroll-view-margin",
  description: `scroll-view 组件的直接子节点 margin 无效`,
  advice: `需要给设置了 margin 的直接子节点套多一层 view。skyline 后续版本考虑从布局算法上支持`,
  level: RuleLevel.Info,
});

const RuleScroolViewWXML = defineRule({ name: "scroll-view-wxml", type: RuleType.WXML }, (ctx) => {
  let scrollViewCount = 0;
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
        ctx.addResultWithPatch(
          {
            ...resultScrollViewImproperType,
            loc: {
              startIndex: start!,
              endIndex: end!,
              path,
            },
          },
          {
            patchedStr: `<scroll-view type="list"`,
            loc: {
              start: node.startIndex!,
              end: node.startIndex! + "<scroll-view".length,
              path: path ?? ctx.env.path,
            },
          }
        );
      }
      if (
        DomUtils.getAttributeValue(node, "scroll-x") === "true" &&
        DomUtils.getAttributeValue(node, "scroll-y") === "true"
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

const RuleScroolViewWXSS = defineRule({ name: "scroll-view-wxss", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node, walkCtx) => {
      if (!isTypeCSS(node, "Declaration") || !node.property.startsWith("margin")) return;
      const wxmlFilename = ctx.getRelatedWXMLFilename();
      const ast = ctx.getRelatedWXMLAst();
      const prelude = walkCtx.rule?.prelude;
      if (!ast || !prelude) return;
      const selector = isTypeCSS(prelude, "Raw") ? prelude.value : formatSelectorList(prelude);
      const children = selectAll(selector, ast);
      for (const child of children) {
        if (child.parent && isType(child.parent, "Tag") && child.parent.name === "scroll-view") {
          const { start, end, path } = getLocationByNode(child);
          ctx.addResult({
            ...resultScrollMargin,
            loc: {
              startIndex: child.startIndex!,
              endIndex: child.endIndex!,
              path: path ?? wxmlFilename ?? null,
            },
          });
        }
      }
    },
  });
});

export default [RuleScroolViewWXML, RuleScroolViewWXSS];
