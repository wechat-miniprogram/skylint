import { RuleType, createResultItem, RuleLevel } from "../interface";
import { generateBasicJsonConfigCheck } from "../templates/json";

const result = createResultItem({
  name: "disable-scroll",
  description: "不支持页面全局滚动",
  advice: `需将页面配置中的 disableScroll 置为 true，并在需要滚动的区域使用 scroll-view 实现，详见文档（链接待定）`,
  patchHint: `将配置项 disableScroll 置为 true`,
  fixable: true,
  level: RuleLevel.Error,
});

export default generateBasicJsonConfigCheck(
  { name: "disable-scroll", type: RuleType.JSON },
  { result, key: "disableScroll", value: true }
);
