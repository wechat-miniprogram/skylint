import { RuleType, RuleLevel, createResultItem } from "../interface";
import { generateBasicJsonConfigCheck } from "../templates/json";

const result = createResultItem({
  subname: "",
  description: "未开启 skyline 渲染",
  advice: `将配置项 renderer 置为 "skyline"`,
  fixable: true,
  level: RuleLevel.Error,
  // patchHint: `将配置项 disableScroll 置为 true`,
});

export default generateBasicJsonConfigCheck(
  { name: "renderer-skyline", type: RuleType.JSON },
  { result, key: "renderer", value: "skyline" }
);
