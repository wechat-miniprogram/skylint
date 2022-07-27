import { parse as parseCSS, CssNode } from "css-tree";
import { DefaultTreeAdapterMap, parseFragment as parseHTML } from "parse5";
import parseJSON, {type ValueNode} from "json-to-ast";
import { Rule, RuleType } from "./rules/interface";
import { walk as walkHTML } from "./walker/html";
import { walk as walkCSS } from "./walker/css";
import { walk as walkJSON } from "./walker/json";
import { Walker } from "./walker/interface";

interface IParseOptions {
  wxml?: string;
  wxss?: string;
  json?: string;
  Rules?: (() => Rule<any>)[];
}

const classifyRules = (Rules: (() => Rule<any>)[]) => {
  const wxmlRules: Rule<RuleType.WXML>[] = [];
  const wxssRules: Rule<RuleType.WXSS>[] = [];
  const nodeRules: Rule<RuleType.Node>[] = [];
  const jsonRules: Rule<RuleType.JSON>[] = [];
  const anyRules: Rule<RuleType.Unknown>[] = [];

  for (const Rule of Rules) {
    const rule = Rule();
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

export const parse = (options: IParseOptions) => {
  const { wxml, wxss, json, Rules = [] } = options;
  const { wxmlRules, wxssRules, nodeRules, jsonRules, anyRules } = classifyRules(Rules);
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
