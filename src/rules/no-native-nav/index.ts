import { RuleType, createResultItem, RuleLevel } from "../interface";
import { generateBasicJsonConfigCheck } from "../templates/json";

const result = createResultItem({
  subname: "",
  description: "不支持的原生导航栏",
  advice: `将配置项 navigationStyle 置为 "custom"，并自行实现自定义导航栏`,
  patchHint: `将配置项 navigationStyle 置为 "custom"`,
  level: RuleLevel.Error,
});

export default generateBasicJsonConfigCheck(
  { name: "no-native-nav", type: RuleType.JSON },
  { result, key: "navigationStyle", value: "custom" }
);
