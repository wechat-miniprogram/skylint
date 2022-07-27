import { parse as parseCSS, CssNode } from "css-tree";
import { DefaultTreeAdapterMap, parseFragment as parseHTML } from "parse5";
import parseJSON, { type ValueNode } from "json-to-ast";
import { Rule, RuleType } from "./rules/interface";
import { walk as walkHTML } from "./walker/html";
import { walk as walkCSS } from "./walker/css";
import { walk as walkJSON } from "./walker/json";
import { Walker } from "./walker/interface";

interface IParseOptions<T> {
  wxml?: string;
  wxss?: string;
  json?: string;
  Rules?: ((env?: T) => Rule<any>)[];
  env?: T;
}

const classifyRules = (rules: Rule<any>[]) => {
  const wxmlRules: Rule<RuleType.WXML>[] = [];
  const wxssRules: Rule<RuleType.WXSS>[] = [];
  const nodeRules: Rule<RuleType.Node>[] = [];
  const jsonRules: Rule<RuleType.JSON>[] = [];
  const anyRules: Rule<RuleType.Unknown>[] = [];

  for (const rule of rules) {
    anyRules.push(rule);
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
      case RuleType.JSON:
        jsonRules.push(rule);
        continue;
      default:
        break;
    }
  }
  return { wxmlRules, wxssRules, nodeRules, jsonRules, anyRules };
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

export const parse = <T>(options: IParseOptions<T>) => {
  const { wxml, wxss, json, Rules = [], env } = options;
  const rules = Rules.map((Rule) => Rule(env)); // inject env into rules
  const { wxmlRules, wxssRules, nodeRules, jsonRules, anyRules } = classifyRules(rules);
  let astWXML: DefaultTreeAdapterMap["documentFragment"] | undefined;
  let astWXSS: CssNode | undefined;
  let astJSON: ValueNode | undefined;
  if (wxml) {
    astWXML = parseHTML(wxml, { sourceCodeLocationInfo: true });
    runLifetimeHooks(wxmlRules, astWXML, walkHTML);
  }

  if (wxss) {
    astWXSS = parseCSS(wxss, { positions: true });
    runLifetimeHooks(wxssRules, astWXSS, walkCSS);
  }

  if (json) {
    astJSON = parseJSON(json);
    runLifetimeHooks(jsonRules, astJSON, walkJSON);
  }

  const ruleResults: Pick<Rule<any>, "name" | "level" | "results" | "patches">[] = [];
  anyRules.forEach((rule) => {
    const { name, level, results, patches } = rule;
    if (results.length) ruleResults.push({ name, level, results, patches });
  });

  return { ruleResults, astWXML, astWXSS, astJSON };
};
