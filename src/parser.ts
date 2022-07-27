import { parse as parseCSS, Rule as CSSRule } from "css-tree";
import { parseFragment as parseHTML } from "parse5";
import { Rule, RuleResultItem, RuleType } from "./rules/interface";
import { walk as walkHTML } from "./walker/html";
import { walk as walkCSS } from "./walker/css";

import ruleBoxSizing from "./rules/box-sizing";
import ruleDisplayFlex from "./rules/display-flex";
import ruleDisplayInline from "./rules/display-inline";
import ruleDisplayInlineBlock from "./rules/display-inline-block";
import ruleNoCalc from "./rules/no-calc";
import ruleNoPseudo from "./rules/no-pseudo";
import rulePositionFixed from "./rules/position-fixed";
import ruleScrollView from "./rules/scroll-view";

interface IParseOptions {
  wxml?: string;
  wxss?: string;
  rules?: Rule<any>[];
}

export const parse = (options: IParseOptions) => {
  const { wxml = "", wxss = "", rules = [] } = options;
  const wxmlRules: Rule<RuleType.WXML>[] = [];
  const wxssRules: Rule<RuleType.WXSS>[] = [];
  const nodeRules: Rule<RuleType.Node>[] = [];
  for (const rule of rules) {
    switch (rule.type) {
      case RuleType.WXML:
        wxmlRules.push(rule);
        continue;
      case RuleType.WXSS:
        wxssRules.push(rule);
        continue;
      case RuleType.Node:
        nodeRules.push(rule);
        continue;
      default:
        break;
    }
  }
  const astWXSS = parseCSS(wxss, { positions: true });
  const astWXML = parseHTML(wxml, { sourceCodeLocationInfo: true });

  rules.forEach((rule) => rule.before?.());

  walkHTML(astWXML, (node) => {
    wxmlRules.forEach((rule) => {
      rule.onVisit?.(node);
    });
  });
  walkCSS(astWXSS, (node) => {
    wxssRules.forEach((rule) => {
      rule.onVisit?.(node);
    });
  });

  const ret: Pick<Rule<any>, "name" | "level" | "results">[] = [];
  rules.forEach((rule) => {
    rule.after?.();
    const { name, level, results } = rule;
    if (results.length) ret.push({ name, level, results });
  });

  return ret;
};
