import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

const result = createResultItem({
  subname: "",
  description: "暂不支持 darkmode",
  advice: `目前只能通过 wx.onThemeChange 接口监听系统 darkmode 切换，自行通过切换 class 的方式实现。skyline 后续版本会支持`,
  level: RuleLevel.Info,
});

export default defineRule({ name: "darkmode", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType(node, "MediaFeature") && node.name === "prefers-color-scheme") {
        const loc = node.loc!;
        ctx.addResult({
          ...result,
          loc: {
            startLn: loc.start.line,
            endLn: loc.end.line,
            startCol: loc.start.column,
            endCol: loc.end.column,
            path: ctx.env.path,
          },
        });
      }
    },
  });
});
