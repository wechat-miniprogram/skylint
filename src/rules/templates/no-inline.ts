import { hasChildren } from "domhandler";
import { getLocationByNode } from "src/utils/dom-ast";
import { isType } from "src/walker/html";
import { defineRule, RuleType, RuleResultItem, RuleBasicInfoWithOptionalLevel } from "../interface";

interface BasicNoInlineCheckOptions {
  result: RuleResultItem;
  tagName?: string;
  parentTagNameShouldBe?: string;
}

export const generateNoInlineTagChildrenCheck = (
  info: RuleBasicInfoWithOptionalLevel<RuleType.WXML>,
  { result, tagName, parentTagNameShouldBe }: BasicNoInlineCheckOptions
) =>
  defineRule(info, (ctx) => {
    ctx.lifetimes({
      onVisit: (node) => {
        if (!isType(node, "Tag") || (tagName && node.name !== tagName)) return;
        if (!hasChildren(node)) return;
        let isPrevText = false;
        for (const child of node.childNodes) {
          const isText =
            (isType(child, "Text") && child.data.trim() !== "") || (isType(child, "Tag") && child.name === "text");
          if (isText && isPrevText && (parentTagNameShouldBe ? node.name !== parentTagNameShouldBe : true)) {
            const { start, end, path } = getLocationByNode(node);
            ctx.addResult({
              ...result,
              loc: {
                startIndex: start!,
                endIndex: end!,
                path,
              },
            });
            break;
          }
          isPrevText = isText;
        }
      },
    });
  });
