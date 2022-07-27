import { hasChildren } from "domhandler";
import { isType } from "src/walker/html";
import { defineRule, RuleType, RuleResultItem, RuleBasicInfoWithOptionalLevel } from "../interface";

interface BasicNoInlineCheckOptions {
  result: RuleResultItem;
  tagName: string;
}

export const generateNoInlineTagChildrenCheck = (
  info: RuleBasicInfoWithOptionalLevel<RuleType.WXML>,
  { result, tagName }: BasicNoInlineCheckOptions
) =>
  defineRule(info, (ctx) => {
    ctx.lifetimes({
      onVisit: (node) => {
        if (!isType(node, "Tag") || node.name !== tagName) return;
        if (!hasChildren(node)) return;
        let isPrevText = true;
        for (const child of node.childNodes) {
          const isText =
            (isType(child, "Text") && child.data.trim() !== "") || (isType(child, "Tag") && child.name === "text");
          if (isText !== isPrevText) {
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
