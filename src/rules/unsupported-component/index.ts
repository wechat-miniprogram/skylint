import { isType } from "src/walker/html";
import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";

const results = {
  "movable-view": createResultItem({
    subname: "movable-view",
    description: "不支持 movable-view 组件",
    advice: "不再支持 movable-view 组件，通过 skyline 的新特性，worklet 动画 + 手势系统实现",
    level: RuleLevel.Error,
  }),
  ad: createResultItem({
    subname: "ad",
    description: "暂不支持 ad 组件",
    advice: "skyline 后续版本会支持",
    level: RuleLevel.Verbose,
  }),
  "picker-view": createResultItem({
    subname: "picker-view",
    description: "暂不支持 picker-view 组件",
    advice: "skyline 后续版本会支持",
    level: RuleLevel.Error,
  }),
  form: createResultItem({
    subname: "form",
    description: "暂不支持 form 组件",
    advice: "skyline 后续版本会支持",
    level: RuleLevel.Error,
  }),
  video: createResultItem({
    subname: "video",
    description: "暂只支持基础播放功能",
    advice: "完整功能 skyline 后续版本会支持",
    level: RuleLevel.Verbose,
  }),
  icon: createResultItem({
    subname: "icon",
    description: "暂不支持 icon 组件",
    advice:
      "不再支持 icon 组件，可引入 weui 组件库实现 https://github.com/wechat-miniprogram/weui-miniprogram/tree/feat-skyline",
    level: RuleLevel.Error,
  }),
  switch: createResultItem({
    subname: "switch",
    description: "不支持 switch 组件",
    advice:
      "不再支持 switch 组件，可引入 weui 组件库实现 https://github.com/wechat-miniprogram/weui-miniprogram/tree/feat-skyline",
    level: RuleLevel.Error,
  }),
};

export default defineRule({ name: "unsupported-component", type: RuleType.WXML }, (ctx) => {
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
});
