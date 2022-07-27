import { RuleType, createResultItem, RuleLevel } from "../interface";
import { generateBasicJsonConfigCheck } from "../templates/json";

const result = createResultItem({
  subname: "",
  description: "暂不支持 darkmode",
  advice: `目前只能通过 wx.onThemeChange 接口监听系统 darkmode 切换，自行通过切换 class 的方式实现。skyline 后续版本会支持`,
  level: RuleLevel.Info,
});

export default generateBasicJsonConfigCheck(
  { name: "darkmode", type: RuleType.JSON },
  { result, key: "darkmode", value: false, allowUndefined: true, autoPatch: false }
);
