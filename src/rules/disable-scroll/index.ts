import { RuleType, createResultItem, RuleLevel } from "../interface";
import { generateBasicJsonConfigCheck } from "../templates/json";

const result = createResultItem({
  subname: "",
  description: "不支持的页面全局滚动",
  advice: `将配置项 disableScroll 置为 true，在需要滚动的区域使用 scroll-view 实现`,
  patchHint: `将配置项 disableScroll 置为 true`,
  level: RuleLevel.Error,
});

export default generateBasicJsonConfigCheck(
  { name: "disable-scroll", type: RuleType.JSON },
  { result, key: "disableScroll", value: true }
);
