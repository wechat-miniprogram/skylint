import { RuleType, createResultItem, RuleLevel } from "../interface";
import { generateNoInlineTagChildrenCheck } from "../templates/no-inline";

const result = createResultItem({
  name: "no-inline-text",
  description: "多段文本内联只能使用 text/span 组件包裹",
  advice:
    "目前不支持 inline ，需通过 text 组件实现，如 <view> foo <text>bar</text> </view> 要改为 <text> foo <text>bar</text> </text>",
  level: RuleLevel.Error,
  withCodeFrame: true,
});

export default generateNoInlineTagChildrenCheck(
  { name: "no-inline-text", type: RuleType.WXML },
  { result, parentTagNameShouldBe: ["text", "span"] }
);
