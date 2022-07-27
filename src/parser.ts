import { parse as parseCSS } from "css-tree";
import { parseFragment as parseHTML } from "parse5";
import { Rule, RuleType } from "./rules/interface";
import { walk as walkHTML } from "./walker/html";
import { walk as walkCSS } from "./walker/css";
import { Walker } from "./walker/interface";

interface IParseOptions {
  wxml?: string;
  wxss?: string;
  rules?: Rule<any>[];
}

const classifyRules = (rules: Rule<any>[]) => {
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
  return { wxmlRules, wxssRules, nodeRules };
};

const runLifetimeHooks = <T>(rules: Rule<any>[], ast: any, walker: Walker<T>) => {
  rules.forEach((rule) => rule.before?.());
  walker(ast, (node) => {
    rules.forEach((rule) => {
      rule.onVisit?.(node);
    });
  });
  rules.forEach((rule) => rule.after?.());
};

export const parse = (options: IParseOptions) => {
  const { wxml, wxss, rules = [] } = options;
  const { wxmlRules, wxssRules, nodeRules } = classifyRules(rules);

  if (wxml) {
    const astWXML = parseHTML(wxml, { sourceCodeLocationInfo: true });
    runLifetimeHooks(wxmlRules, astWXML, walkHTML);
  }

  if (wxss) {
    const astWXSS = parseCSS(wxss, { positions: true });
    runLifetimeHooks(wxssRules, astWXSS, walkCSS);
  }

  const ret: Pick<Rule<any>, "name" | "level" | "results">[] = [];
  rules.forEach((rule) => {
    const { name, level, results } = rule;
    if (results.length) ret.push({ name, level, results });
  });

  return ret;
};
