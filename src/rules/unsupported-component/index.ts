import { isType } from "src/walker/html";
import {
  defineRule,
  RuleType,
  createResultItem,
  RuleLevel,
} from "../interface";

const results = {
  "movable-view": createResultItem({
    name: "movable-view",
    description: "不支持 movable-view 组件",
    advice:
      "不再支持 movable-view 组件，通过 skyline 的新特性，worklet 动画 + 手势系统实现",
    level: RuleLevel.Error,
  }),
  video: createResultItem({
    name: "video",
    description: "暂只支持基础播放功能",
    advice: "完整功能 skyline 后续版本会支持",
    level: RuleLevel.Verbose,
  }),
  form: createResultItem({
    name: "form",
    description: "不支持 form 组件",
    advice: "完整功能 skyline 后续版本会支持",
    level: RuleLevel.Verbose,
  }),
};

export default defineRule(
  { name: "unsupported-component", type: RuleType.WXML },
  (ctx) => {
    ctx.lifetimes({
      onVisit: (node) => {
        if (!isType(node, "Tag")) return;
        if (Reflect.has(results, node.name)) {
          ctx.addResult({
            ...results[node.name as keyof typeof results],
            loc: {
              startIndex: node.startIndex!,
              endIndex: node.endIndex!,
              path: ctx.env.path,
            },
          });
        }
      },
    });
  }
);
