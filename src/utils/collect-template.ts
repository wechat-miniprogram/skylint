import { readFileSync } from "fs";
import { BasicParseEnv, parse } from "src/parser";
import { defineRule, RuleType } from "src/rules/interface";
import { isType } from "src/walker/html";
import { ParentNode } from "domhandler";
import { resolvePath } from "./resolve";
import { replaceChildWithChildren } from "./dom-ast";

interface FragmentInfo {
  fragment: ParentNode;
  fromFile: string;
}

interface CollectTemplateEnv extends BasicParseEnv {
  rootPath: string;
  wxmlPaths: string[];
  tmplFragments: Map<string, FragmentInfo>;
  importFragments: Map<string, FragmentInfo>;
  includeFragments: Map<string, FragmentInfo>;
}

// TODO avoid name conflict
const getUniqueKey = (path: string, tmplName: string) => `${tmplName}`;

// TODO scope of import and include
const Rule = defineRule<CollectTemplateEnv, RuleType.WXML>(
  { name: "collect-template", type: RuleType.WXML },
  (ctx) => {
    ctx.lifetimes({
      onVisit: (node, walkerContext) => {
        if (!isType(node, "Tag")) return;
        if (node.name === "template") {
          // <template is="tmpl"/>
          const { is, name } = node.attribs;
          if (is) {
            const key = getUniqueKey(ctx.env.path, is);
            const { fragment, fromFile } = ctx.env.tmplFragments.get(key) ?? {};
            if (!fragment) return;
            replaceChildWithChildren(node, fragment.childNodes, {
              attachFilename: fromFile ?? ctx.env.path,
            });
          } else if (name) {
            const key = getUniqueKey(ctx.env.path, name);
            if (ctx.env.tmplFragments.has(key)) return;
            ctx.env.tmplFragments.set(key, {
              fragment: node,
              fromFile: ctx.env.path,
            });
            replaceChildWithChildren(node, []);
          }
        } else if (node.name === "include") {
          // <include src="header.wxml"/>
          const { src } = node.attribs;
          if (!src) return;
          const srcPath = resolvePath(ctx.env.path, ctx.env.rootPath, src);
          let { fragment: srcAST, fromFile } =
            ctx.env.includeFragments.get(srcPath) ?? {};
          if (!srcAST) [srcAST] = collectTemplate([srcPath], ctx.env);
          // naivePrint(srcAST);
          replaceChildWithChildren(node, srcAST.childNodes, {
            attachFilename: srcPath,
          });
        } else if (node.name === "import") {
          // <import src="header.wxml"/>
          const { src } = node.attribs;
          const srcPath = resolvePath(ctx.env.path, ctx.env.rootPath, src);
          let { fragment: srcAST, fromFile } =
            ctx.env.importFragments.get(srcPath) ?? {};
          if (!srcAST) [srcAST] = collectTemplate([srcPath], ctx.env);
          replaceChildWithChildren(node, []);
        }
      },
    });
  }
);

export const collectTemplate = (
  wxmlPaths: string[],
  env?: CollectTemplateEnv
) => {
  const originalPaths = [...wxmlPaths];
  const newEnv = env ?? {
    rootPath: "",
    path: "",
    wxmlPaths,
    importFragments: new Map(),
    includeFragments: new Map(),
    tmplFragments: new Map(),
  };

  return wxmlPaths.map((path) => {
    const wxml = readFileSync(path).toString();
    let { astWXML } = parse({ wxml, Rules: [Rule], env: { ...newEnv, path } });
    return astWXML!;
  });
};
