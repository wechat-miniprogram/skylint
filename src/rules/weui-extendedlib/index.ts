import { isType } from "src/walker/json";
import { isType as isTypeCSS } from "src/walker/css";
import { defineRule, RuleType, RuleResultItem, createResultItem, RuleLevel } from "../interface";
import { BasicParseEnv } from "src/parser";

const enum State {
  Unknown,
  WXSS,
  JSON,
}

const stateMap = new Map<string, State>();

const result = createResultItem({
  subname: "",
  description: "暂不支持 weui 扩展库",
  advice: "目前 skyline 页面需要通过引入 weui npm 包的方式使用，后续版本会支持，webview 页面则不影响",
  level: RuleLevel.Error,
});

const before = (env?: BasicParseEnv) => {
  if (!env || !env.path) return;
  const state = stateMap.get(env.path);
  if (!state) stateMap.set(env.path, State.Unknown);
};

const RuleWeuiExtendedlibWXSS = defineRule(
  {
    name: "weui-extendedlib",
    type: RuleType.WXSS,
  },
  (ctx) => {
    let weuiImported = false;
    ctx.lifetimes({
      before: () => before(ctx.env),
      onVisit: (node) => {
        if (weuiImported || !ctx.env || !ctx.env.path) return;
        if (
          !isTypeCSS(node, "Atrule") ||
          node.name !== "import" ||
          !node.prelude ||
          !isTypeCSS(node.prelude, "AtrulePrelude")
        ) {
          return;
        }
        const imported = node.prelude.children.some((child) => {
          let path: string | null = null;
          if (isTypeCSS(child, "String")) {
            path = child.value;
          } else if (isTypeCSS(child, "Url") && isTypeCSS(child.value, "String")) {
            path = child.value.value;
          }
          if (path?.endsWith("weui-miniprogram/weui-wxss/dist/style/weui.wxss")) return true;
          return false;
        });
        if (imported) weuiImported = imported;
      },
      after: () => {
        if (!ctx.env || !ctx.env.path) return;
        const state = stateMap.get(ctx.env.path);
        if (state === undefined) return;
        if (state === State.JSON) {
          stateMap.delete(ctx.env.path);
          !weuiImported &&
            ctx.addResult({
              ...result,
            });
        } else {
          stateMap.set(ctx.env.path, State.WXSS);
        }
      },
    });
  }
);

const RuleWeuiExtendedlibJSON = defineRule(
  {
    name: "weui-extendedlib",
    type: RuleType.JSON,
  },
  (ctx) => {
    let weuiUsed = false;
    ctx.lifetimes({
      before: () => before(ctx.env),
      onVisit: (node) => {
        if (weuiUsed || !isType(node, "Property")) return;
        if (node.key.value !== "usingComponents" || !isType(node.value, "Object")) return;
        for (const child of node.value.children) {
          if (!isType(child, "Property") || !isType(child.value, "Literal")) continue;
          if (typeof child.value.value === "string" && child.value.value.match(/weui-miniprogram\//)) {
            weuiUsed = true;
            break;
          }
        }
      },
      after() {
        if (!ctx.env || !ctx.env.path) return;
        const state = stateMap.get(ctx.env.path);
        if (state === undefined) return;
        if (state === State.WXSS) {
          stateMap.delete(ctx.env.path);
          !weuiUsed &&
            ctx.addResult({
              ...result,
            });
        } else {
          stateMap.set(ctx.env.path, State.JSON);
        }
      },
    });
  }
);

export default [RuleWeuiExtendedlibWXSS, RuleWeuiExtendedlibJSON];
