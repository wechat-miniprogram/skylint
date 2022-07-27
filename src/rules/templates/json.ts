import { isType } from "src/walker/json";
import { ObjectNode, PropertyNode } from "json-to-ast";
import { defineRule, RuleType, RuleResultItem, RuleBasicInfoWithOptionalLevel } from "../interface";

interface BasicJsonConfigCheckOptions {
  result: RuleResultItem;
  key: string;
  value: string | number | boolean | null;
}

export const generateBasicJsonConfigCheck = (
  info: RuleBasicInfoWithOptionalLevel<RuleType.JSON>,
  { result, key, value }: BasicJsonConfigCheckOptions
) =>
  defineRule(info, (ctx) => {
    let firstNode: ObjectNode | null = null;
    let propNode: PropertyNode | null = null;
    let flag = false;
    ctx.lifetimes({
      onVisit: (node) => {
        if (!firstNode && isType(node, "Object")) firstNode = node;
        if (!isType(node, "Property")) return;
        if (node.key.value !== key) return;
        propNode = node;
        if (isType(node.value, "Literal") && node.value.value === value) flag = true;
      },
      after: () => {
        if (flag) return;
        const node = propNode;
        if (node) {
          ctx.addResult({
            ...result,
            loc: {
              startLn: node.loc!.start.line,
              endLn: node.loc!.end.line,
              startCol: node.loc!.start.column,
              endCol: node.loc!.end.column,
            },
          });
          ctx.addPatch({
            patchedStr: JSON.stringify(value),
            loc: {
              start: node.value.loc!.start.offset,
              end: node.value.loc!.end.offset,
            },
          });
        } else if (firstNode) {
          ctx.addResult({
            ...result,
          });
          ctx.addPatch({
            patchedStr: `"${key}": ${JSON.stringify(value)},`,
            loc: {
              start: firstNode.loc!.start.offset + 1,
              end: firstNode.loc!.start.offset + 1,
            },
          });
        }
      },
    });
  });
