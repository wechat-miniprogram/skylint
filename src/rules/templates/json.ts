import { isType } from "src/walker/json";
import { ObjectNode, PropertyNode } from "json-to-ast";
import { defineRule, RuleType, RuleResultItem, RuleBasicInfoWithOptionalLevel } from "../interface";

type JSONLiterial = string | number | boolean | null;

interface BasicJsonConfigCheckOptions {
  result: RuleResultItem;
  key: string;
  value: JSONLiterial | JSONLiterial[];
  /** @default true */
  autoPatch?: boolean;
  /** @default false */
  allowUndefined?: boolean;
}

const enum State {
  Undefined,
  Unequal,
  Equal,
}

export const generateBasicJsonConfigCheck = (
  info: RuleBasicInfoWithOptionalLevel<RuleType.JSON>,
  { result, key, value, autoPatch = true, allowUndefined }: BasicJsonConfigCheckOptions
) =>
  defineRule(info, (ctx) => {
    let firstNode: ObjectNode | null = null;
    let propNode: PropertyNode | null = null;
    let state: State = State.Undefined;
    ctx.lifetimes({
      onVisit: (node) => {
        if (!firstNode && isType(node, "Object")) firstNode = node;
        if (!isType(node, "Property")) return;
        if (node.key.value !== key) return;
        propNode = node;
        if (isType(node.value, "Literal")) {
          state = (Array.isArray(value) ? value.includes(node.value.value) : node.value.value === value)
            ? State.Equal
            : State.Unequal;
        }
      },
      after: () => {
        if ((allowUndefined && state === State.Undefined) || State.Equal) return;
        const node = propNode;
        if (node) {
          ctx.addResult({
            ...result,
            loc: {
              startLn: node.loc!.start.line,
              endLn: node.loc!.end.line,
              startCol: node.loc!.start.column,
              endCol: node.loc!.end.column,
              path: ctx.env.path,
            },
          });
          autoPatch &&
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
          autoPatch &&
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
