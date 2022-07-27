import { RuleType, RuleResultItem, RuleLevel } from "../interface";
import { generateBasicJsonConfigCheck } from "../templates/json";

const result: RuleResultItem = {
  subname: "",
  description: "未开启 skyline 渲染",
  advice: `将配置项 renderer 置为 "skyline"`,
  // patchHint: `将配置项 disableScroll 置为 true`,
};

export default generateBasicJsonConfigCheck(
  { name: "renderer-skyline", type: RuleType.JSON, level: RuleLevel.Error },
  { result, key: "renderer", value: "skyline" }
);
