import { RuleType, createResultItem, RuleLevel } from "../interface";
import { generateNoInlineTagChildrenCheck } from "../templates/no-inline";

const result = createResultItem({
  name: "navigator",
  description: "navigator 组件只能嵌套文本",
  advice: "同 text 组件，只支持内联文本，若需要实现块级元素，可改为 button 实现",
  level: RuleLevel.Warn,
});

export default generateNoInlineTagChildrenCheck(
  { name: "navigator", type: RuleType.WXML },
  { result, tagName: "navigator" }
);
