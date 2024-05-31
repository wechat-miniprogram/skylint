#!/usr/bin/env node
import { Command } from 'commander';
import { argv, cwd, stdout } from 'process';
import { globby } from 'globby';
import { readFile, writeFile } from 'fs/promises';
import chalk from 'chalk';
import { d as defineRule, i as isType, c as createResultItem, R as RuleLevel, a as RuleType, b as isType$1, e as isType$2, p as parse, f as applyPatchesOnString } from './chunks/parser.mjs';
import { isText, Text, isComment, Comment, isTag, Element, isCDATA, CDATA, isDocument, Document, isDirective, ProcessingInstruction, hasChildren } from 'domhandler';
import * as CssTree from 'css-tree';
import { selectAll } from 'css-select';
import { DomUtils } from 'htmlparser2';
import { format } from 'util';
import inquirer from 'inquirer';
import path, { join, resolve, dirname, relative } from 'path';
import { existsSync, readFileSync, lstatSync } from 'fs';
import { codeFrameColumns } from '@babel/code-frame';
import lineColumn from 'line-column';
import 'json-to-ast';
import 'magic-string';

const name = "skylint";
const version = "1.0.2";
const description = "Skyline 小程序迁移工具. Migration assistant for Skyline miniapp.";
const main$1 = "dist/index.mjs";
const type = "module";
const scripts = {
	build: "unbuild",
	start: "node dist/cli.mjs",
	test: "NODE_OPTIONS='--experimental-vm-modules' jest"
};
const bin = {
	skylint: "./dist/cli.mjs"
};
const keywords = [
	"skyline",
	"linter",
	"migration"
];
const author = "maniacata";
const license = "MIT";
const files = [
	"./dist",
	"./LICENSE",
	"./third-party-licenses.txt"
];
const dependencies = {
	"@babel/code-frame": "^7.18.6",
	"@babel/preset-typescript": "^7.18.6",
	acorn: "^8.7.1",
	chalk: "^5.0.1",
	commander: "^9.4.0",
	"css-select": "^5.1.0",
	"css-tree": "^2.1.0",
	"dom-serializer": "^2.0.0",
	domhandler: "^5.0.3",
	domutils: "^3.0.1",
	globby: "^13.1.2",
	htmlparser2: "^8.0.1",
	inquirer: "^9.0.1",
	jest: "^28.1.3",
	"json-to-ast": "^2.1.0",
	"line-column": "^1.0.2",
	"magic-string": "^0.26.2",
	parse5: "^7.0.0",
	"ts-node": "^10.9.1",
	typescript: "^4.7.4",
	unbuild: "^0.7.4"
};
const devDependencies = {
	"@babel/core": "^7.18.9",
	"@babel/preset-env": "^7.18.9",
	"@types/babel__code-frame": "^7.0.3",
	"@types/inquirer": "^8.2.1",
	"@types/json-to-ast": "^2.1.2",
	"@types/css-tree": "^1.0.7",
	"@types/line-column": "^1.0.0",
	"@types/node": "^18.0.6",
	"babel-jest": "^28.1.3"
};
const pkg = {
	name: name,
	version: version,
	description: description,
	main: main$1,
	type: type,
	scripts: scripts,
	bin: bin,
	keywords: keywords,
	author: author,
	license: license,
	files: files,
	dependencies: dependencies,
	devDependencies: devDependencies
};

const nodeFilenameMap = /* @__PURE__ */ new WeakMap();
const attachFilenameToNode = (node, filename) => {
  nodeFilenameMap.set(node, filename);
};
const getLocationByNode = (node) => {
  const { startIndex: start, endIndex: end } = node;
  const path = nodeFilenameMap.get(node) ?? null;
  return { start, end, path };
};
function cloneNode(node, recursive = false, transform) {
  let result;
  if (isText(node)) {
    result = new Text(node.data);
  } else if (isComment(node)) {
    result = new Comment(node.data);
  } else if (isTag(node)) {
    const children = recursive ? cloneChildren(node.children, transform) : [];
    const clone = new Element(node.name, { ...node.attribs }, children);
    children.forEach((child) => child.parent = clone);
    if (node.namespace != null) {
      clone.namespace = node.namespace;
    }
    if (node["x-attribsNamespace"]) {
      clone["x-attribsNamespace"] = { ...node["x-attribsNamespace"] };
    }
    if (node["x-attribsPrefix"]) {
      clone["x-attribsPrefix"] = { ...node["x-attribsPrefix"] };
    }
    result = clone;
  } else if (isCDATA(node)) {
    const children = recursive ? cloneChildren(node.children) : [];
    const clone = new CDATA(children);
    children.forEach((child) => child.parent = clone);
    result = clone;
  } else if (isDocument(node)) {
    const children = recursive ? cloneChildren(node.children) : [];
    const clone = new Document(children);
    children.forEach((child) => child.parent = clone);
    if (node["x-mode"]) {
      clone["x-mode"] = node["x-mode"];
    }
    result = clone;
  } else if (isDirective(node)) {
    const instruction = new ProcessingInstruction(node.name, node.data);
    if (node["x-name"] != null) {
      instruction["x-name"] = node["x-name"];
      instruction["x-publicId"] = node["x-publicId"];
      instruction["x-systemId"] = node["x-systemId"];
    }
    result = instruction;
  } else {
    throw new Error(`Not implemented yet: ${node.type}`);
  }
  result.startIndex = node.startIndex;
  result.endIndex = node.endIndex;
  if (node.sourceCodeLocation != null) {
    result.sourceCodeLocation = node.sourceCodeLocation;
  }
  transform?.(result);
  return result;
}
function cloneChildren(childs, transform) {
  const children = childs.map((child) => cloneNode(child, true, transform));
  for (let i = 1; i < children.length; i++) {
    children[i].prev = children[i - 1];
    children[i - 1].next = children[i];
  }
  return children;
}
const replaceChildWithChildren = (child, children, option = {}) => {
  const parent = child.parentNode;
  if (!parent)
    return false;
  const { transform, keepLocation = true, attachFilename = false } = option;
  parent.children = parent.children.flatMap((childNode) => {
    if (childNode === child) {
      const newChildren = cloneChildren(children, (newChild) => {
        if (keepLocation) {
          if (attachFilename)
            attachFilenameToNode(newChild, attachFilename);
        } else {
          newChild.startIndex = newChild.endIndex = null;
        }
        transform?.(newChild);
      });
      newChildren.forEach((child2) => child2.parent = parent);
      const firstChild = newChildren[0];
      const lastChild = newChildren[newChildren.length - 1];
      if (firstChild) {
        if (child.prev)
          child.prev.next = firstChild;
        firstChild.prev = child.prev;
      }
      if (lastChild) {
        if (child.next)
          child.next.prev = lastChild;
        lastChild.next = child.next;
      }
      return newChildren;
    }
    return childNode;
  });
  return true;
};

const matchTagName = (tag, matcher) => {
  if (!matcher)
    return false;
  if (Array.isArray(matcher)) {
    return matcher.some((m) => tag.match(m));
  } else {
    return tag.match(matcher);
  }
};
const generateNoInlineTagChildrenCheck = (info, { result, tagName, parentTagNameShouldBe }) => defineRule(info, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (!isType(node, "Tag") || tagName && node.name !== tagName)
        return;
      if (!hasChildren(node))
        return;
      let isPrevText = false;
      for (const child of node.childNodes) {
        const isText = isType(child, "Text") && child.data.trim() !== "" || isType(child, "Tag") && child.name === "text";
        if (isText && isPrevText && !matchTagName(node.name, parentTagNameShouldBe)) {
          const { start, end, path } = getLocationByNode(node);
          ctx.addResult({
            ...result,
            loc: {
              startIndex: start,
              endIndex: end,
              path
            }
          });
          break;
        }
        isPrevText = isText;
      }
    }
  });
});

const result$9 = createResultItem({
  name: "navigator",
  description: "navigator \u7EC4\u4EF6\u53EA\u80FD\u5D4C\u5957\u6587\u672C",
  advice: "\u540C text \u7EC4\u4EF6\uFF0C\u53EA\u652F\u6301\u5185\u8054\u6587\u672C\uFF0C\u82E5\u9700\u8981\u5B9E\u73B0\u5757\u7EA7\u5143\u7D20\uFF0C\u53EF\u6539\u4E3A button \u5B9E\u73B0",
  level: RuleLevel.Warn
});
const RuleNagivator = generateNoInlineTagChildrenCheck(
  { name: "navigator", type: RuleType.WXML },
  { result: result$9, tagName: "navigator" }
);

const result$8 = createResultItem({
  name: "no-inline-text",
  description: "\u591A\u6BB5\u6587\u672C\u5185\u8054\u53EA\u80FD\u4F7F\u7528 text/span \u7EC4\u4EF6\u5305\u88F9",
  advice: "\u76EE\u524D\u4E0D\u652F\u6301 inline \uFF0C\u9700\u901A\u8FC7 text \u7EC4\u4EF6\u5B9E\u73B0\uFF0C\u5982 <view> foo <text>bar</text> </view> \u8981\u6539\u4E3A <text> foo <text>bar</text> </text>",
  level: RuleLevel.Error,
  withCodeFrame: true
});
const RuleNoInlineText = generateNoInlineTagChildrenCheck(
  { name: "no-inline-text", type: RuleType.WXML },
  { result: result$8, parentTagNameShouldBe: ["text", "span"] }
);

const result$7 = createResultItem({
  name: "no-svg-style-tag",
  description: "\u4E0D\u652F\u6301 svg \u7EC4\u4EF6\u5185\u4F7F\u7528 style \u6807\u7B7E",
  advice: "\u5728 wxss \u6587\u4EF6\u5185\u4E66\u5199\u6837\u5F0F\u89C4\u5219",
  level: RuleLevel.Error,
  withCodeFrame: true
});
const RuleNoSvgStyleTag = defineRule({ name: "no-svg-style-tag", type: RuleType.WXML }, (ctx) => {
  const dfs = (node, ruleCtx) => {
    if (isType(node, "Tag") && node.name === "style" || isType(node, "Style")) {
      ruleCtx.addResult({
        ...result$7,
        loc: {
          startIndex: node.startIndex,
          endIndex: node.endIndex,
          path: ctx.env.path
        }
      });
    }
    if (hasChildren(node)) {
      for (const childNode of node.childNodes) {
        dfs(childNode, ruleCtx);
      }
    }
  };
  ctx.lifetimes({
    onVisit: (node, walkCtx) => {
      if (isType(node, "Tag") && node.name === "svg" && hasChildren(node)) {
        node.childNodes.forEach((childNode) => dfs(childNode, ctx));
      }
    }
  });
});

const results = {
  "movable-view": createResultItem({
    name: "movable-view",
    description: "\u4E0D\u652F\u6301 movable-view \u7EC4\u4EF6",
    advice: "\u4E0D\u518D\u652F\u6301 movable-view \u7EC4\u4EF6\uFF0C\u901A\u8FC7 skyline \u7684\u65B0\u7279\u6027\uFF0Cworklet \u52A8\u753B + \u624B\u52BF\u7CFB\u7EDF\u5B9E\u73B0",
    level: RuleLevel.Error
  }),
  video: createResultItem({
    name: "video",
    description: "\u6682\u53EA\u652F\u6301\u57FA\u7840\u64AD\u653E\u529F\u80FD",
    advice: "\u5B8C\u6574\u529F\u80FD skyline \u540E\u7EED\u7248\u672C\u4F1A\u652F\u6301",
    level: RuleLevel.Verbose
  }),
  form: createResultItem({
    name: "form",
    description: "\u4E0D\u652F\u6301 form \u7EC4\u4EF6",
    advice: "\u5B8C\u6574\u529F\u80FD skyline \u540E\u7EED\u7248\u672C\u4F1A\u652F\u6301",
    level: RuleLevel.Verbose
  })
};
const RuleUnsupportedComponent = defineRule(
  { name: "unsupported-component", type: RuleType.WXML },
  (ctx) => {
    ctx.lifetimes({
      onVisit: (node) => {
        if (!isType(node, "Tag"))
          return;
        if (Reflect.has(results, node.name)) {
          ctx.addResult({
            ...results[node.name],
            loc: {
              startIndex: node.startIndex,
              endIndex: node.endIndex,
              path: ctx.env.path
            }
          });
        }
      }
    });
  }
);

const result$6 = createResultItem({
  name: "display-flex",
  description: "flex \u5E03\u5C40\u4E0B\u672A\u663E\u5F0F\u6307\u5B9A flex-direction",
  advice: "\u76EE\u524D flex-direction \u9ED8\u8BA4\u4E3A column\uFF0C\u800C\u5728 web \u4E0B\u9ED8\u8BA4\u503C\u4E3A row \u6545\u4E00\u822C\u4E0D\u4F1A\u663E\u5F0F\u6307\u5B9A\uFF0C\u56E0\u6B64\u8FD9\u91CC\u9700\u8981\u663E\u5F0F\u6307\u5B9A\u4E3A row",
  fixable: true,
  level: RuleLevel.Warn
});
const RuleDisplayFlex = defineRule({ name: "display-flex", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType$1(node, "Block")) {
        let loc;
        let hasFlexDirection = false;
        node.children.forEach((child) => {
          if (!isType$1(child, "Declaration"))
            return;
          if (child.property === "display" && isType$1(child.value, "Value") && child.value.children.some((val) => isType$1(val, "Identifier") && val.name === "flex")) {
            loc = child.loc;
          }
          if (child.property === "flex-direction")
            hasFlexDirection = true;
        });
        if (loc && !hasFlexDirection) {
          ctx.addResultWithPatch(
            {
              ...result$6,
              loc: {
                startLn: loc.start.line,
                endLn: loc.end.line,
                startCol: loc.start.column,
                endCol: loc.end.column,
                path: ctx.env.path
              }
            },
            {
              loc: {
                start: loc.start.offset,
                end: loc.start.offset,
                path: ctx.env.path
              },
              patchedStr: "flex-direction: row; "
            }
          );
        }
      }
    }
  });
});

const RuleDisplayInline = defineRule({ name: "display-inline", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType$1(node, "Declaration") && node.property === "display" && isType$1(node.value, "Value")) {
        const val = node.value.children.toArray().find((val2) => isType$1(val2, "Identifier") && ["inline", "inline-block"].includes(val2.name));
        if (!val || !isType$1(val, "Identifier"))
          return;
        const loc = node.loc;
        ctx.addResult({
          name: `display-${val.name}`,
          description: `\u4E0D\u652F\u6301 display: ${val.name}`,
          advice: "\u82E5\u662F\u5E03\u5C40\u9700\u8981\uFF0C\u53EF\u6539\u4E3A flex \u5E03\u5C40\u5B9E\u73B0\uFF1B\u82E5\u662F\u5B9E\u73B0\u5185\u8054\u6587\u672C\uFF0C\u53EF\u4F7F\u7528 text \u7EC4\u4EF6",
          level: RuleLevel.Info,
          loc: {
            startLn: loc.start.line,
            endLn: loc.end.line,
            startCol: loc.start.column,
            endCol: loc.end.column,
            path: ctx.env.path
          }
        });
      }
    }
  });
});

const result$5 = createResultItem({
  name: "mark-wx-for",
  description: `\u672A\u6253\u5F00\u6837\u5F0F\u5171\u4EAB\u6807\u8BB0`,
  advice: `\u6BCF\u4E00\u4E2A\u5217\u8868\u9879\u7684\u6837\u5F0F\u662F\u57FA\u672C\u76F8\u540C\u7684\uFF0C\u56E0\u6B64 skyline \u5B9E\u73B0\u4E86\u6837\u5F0F\u5171\u4EAB\u673A\u5236\uFF0C\u53EF\u964D\u4F4E\u6837\u5F0F\u5339\u914D\u7684\u8017\u65F6\uFF0C\u53EA\u9700\u8981\u5217\u8868\u9879\u52A0\u4E2A list-item\uFF0C\u5982 <view wx:for="" list-item />`,
  level: RuleLevel.Verbose
});
const RuleMarkWxFor = defineRule({ name: "mark-wx-for", type: RuleType.WXML }, (ctx) => {
  ctx.lifetimes({
    before: () => {
    },
    onVisit: (node) => {
      if (!isType(node, "Tag") || !Reflect.has(node.attribs, "wx:for"))
        return;
      if (node.name === "block" && hasChildren(node)) {
        for (const childNode of node.childNodes) {
          if (isType(childNode, "Tag") && !Reflect.has(childNode.attribs, "list-item")) {
            ctx.addResult({
              ...result$5,
              loc: {
                startIndex: childNode.startIndex,
                endIndex: childNode.endIndex,
                path: ctx.env.path
              }
            });
          }
        }
      } else if (!Reflect.has(node.attribs, "list-item")) {
        ctx.addResult({
          ...result$5,
          loc: {
            startIndex: node.startIndex,
            endIndex: node.endIndex,
            path: ctx.env.path
          }
        });
      }
    }
  });
});

const result$4 = createResultItem({
  name: "position-fixed",
  description: "position: fixed \u672A\u6307\u5B9A\u5143\u7D20\u504F\u79FB",
  advice: "position-fixed \u9700\u8981\u6307\u5B9A top/bottom left/right\u3002",
  level: RuleLevel.Error
});
const verboseResult = createResultItem({
  name: "position fixed",
  description: "skyline \u6682\u4E0D\u652F\u6301 stacking context",
  advice: "\u8BF7\u81EA\u884C\u786E\u8BA4 fixed \u8282\u70B9\u7684 z-index \u5C42\u7EA7\u662F\u5426\u7B26\u5408\u9884\u671F",
  level: RuleLevel.Info
});
const RulePositionFixed = defineRule(
  { name: "position-fixed", type: RuleType.WXSS },
  (ctx) => {
    ctx.lifetimes({
      onVisit: (node, visitCtx) => {
        if (isType$1(node, "Declaration") && node.property === "position" && isType$1(node.value, "Value") && node.value.children.some(
          (val) => isType$1(val, "Identifier") && val.name === "fixed"
        )) {
          const loc = node.loc;
          ctx.addResult({
            ...verboseResult,
            loc: {
              startLn: loc.start.line,
              endLn: loc.end.line,
              startCol: loc.start.column,
              endCol: loc.end.column,
              path: ctx.env.path
            }
          });
          const hasLOrR = CssTree.find(visitCtx.block, (node2, item, list) => {
            return isType$1(node2, "Declaration") && (node2.property === "left" || node2.property === "right");
          });
          const hasTOrB = CssTree.find(visitCtx.block, (node2, item, list) => {
            return isType$1(node2, "Declaration") && (node2.property === "top" || node2.property === "bottom");
          });
          if (!hasLOrR || !hasTOrB) {
            ctx.addResult({
              ...result$4,
              loc: {
                startLn: loc.start.line,
                endLn: loc.end.line,
                startCol: loc.start.column,
                endCol: loc.end.column,
                path: ctx.env.path
              }
            });
          }
        }
      }
    });
  }
);

const result$3 = createResultItem({
  name: "text-overflow-ellipse",
  description: "text-overflow: ellipse \u53EA\u5728 text \u7EC4\u4EF6\u4E0B\u751F\u6548",
  advice: "\u6587\u672C\u7701\u7565\u9700\u8981\u901A\u8FC7 text \u7EC4\u4EF6 + text-overflow: ellipse + overflow: hidden \u4E09\u8005\u5B9E\u73B0",
  level: RuleLevel.Warn
});
const RuleTextOverflowEllipse = defineRule({ name: "text-overflow-ellipse", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node, walkCtx) => {
      if (isType$1(node, "Declaration") && node.property === "text-overflow") {
        const loc = node.loc;
        ctx.addResult({
          ...result$3,
          loc: {
            startLn: loc.start.line,
            endLn: loc.end.line,
            startCol: loc.start.column,
            endCol: loc.end.column,
            path: ctx.env.path
          }
        });
      }
    }
  });
});

const generateBasicJsonConfigCheck = (info, { result, key, value, autoPatch = true, allowUndefined }) => defineRule(info, (ctx) => {
  let firstNode = null;
  let propNode = null;
  let state = 0 /* Undefined */;
  ctx.lifetimes({
    onVisit: (node) => {
      if (!firstNode && isType$2(node, "Object"))
        firstNode = node;
      if (!isType$2(node, "Property"))
        return;
      if (node.key.value !== key)
        return;
      propNode = node;
      if (isType$2(node.value, "Literal")) {
        state = (Array.isArray(value) ? value.includes(node.value.value) : node.value.value === value) ? 2 /* Equal */ : 1 /* Unequal */;
      }
    },
    after: () => {
      if (allowUndefined && state === 0 /* Undefined */ || 2 /* Equal */)
        return;
      const node = propNode;
      if (node) {
        const res = {
          ...result,
          loc: {
            startLn: node.loc.start.line,
            endLn: node.loc.end.line,
            startCol: node.loc.start.column,
            endCol: node.loc.end.column,
            path: ctx.env.path
          }
        };
        const patch = {
          patchedStr: JSON.stringify(value),
          loc: {
            start: node.value.loc.start.offset,
            end: node.value.loc.end.offset,
            path: ctx.env.path
          }
        };
        autoPatch ? ctx.addResultWithPatch(res, patch) : ctx.addResult(res);
      } else if (firstNode) {
        const res = { ...result };
        const patch = {
          patchedStr: `"${key}": ${JSON.stringify(value)},`,
          loc: {
            start: firstNode.loc.start.offset + 1,
            end: firstNode.loc.start.offset + 1,
            path: ctx.env.path
          }
        };
        autoPatch ? ctx.addResultWithPatch(res, patch) : ctx.addResult(res);
      }
    }
  });
});

const result$2 = createResultItem({
  name: "no-native-nav",
  description: "\u4E0D\u652F\u6301\u7684\u539F\u751F\u5BFC\u822A\u680F",
  advice: `\u9700\u5C06\u9875\u9762\u914D\u7F6E\u4E2D\u7684 navigationStyle \u7F6E\u4E3A custom\uFF0C\u5E76\u81EA\u884C\u5B9E\u73B0\u81EA\u5B9A\u4E49\u5BFC\u822A\u680F\uFF0C\u8BE6\u89C1\u6587\u6863\uFF08\u94FE\u63A5\u5F85\u5B9A\uFF09`,
  patchHint: `\u5C06\u914D\u7F6E\u9879 navigationStyle \u7F6E\u4E3A "custom"`,
  fixable: true,
  level: RuleLevel.Error
});
const RuleNoNativeNav = generateBasicJsonConfigCheck(
  { name: "no-native-nav", type: RuleType.JSON },
  { result: result$2, key: "navigationStyle", value: "custom" }
);

const result$1 = createResultItem({
  name: "disable-scroll",
  description: "\u4E0D\u652F\u6301\u9875\u9762\u5168\u5C40\u6EDA\u52A8",
  advice: `\u9700\u5C06\u9875\u9762\u914D\u7F6E\u4E2D\u7684 disableScroll \u7F6E\u4E3A true\uFF0C\u5E76\u5728\u9700\u8981\u6EDA\u52A8\u7684\u533A\u57DF\u4F7F\u7528 scroll-view \u5B9E\u73B0\uFF0C\u8BE6\u89C1\u6587\u6863\uFF08\u94FE\u63A5\u5F85\u5B9A\uFF09`,
  patchHint: `\u5C06\u914D\u7F6E\u9879 disableScroll \u7F6E\u4E3A true`,
  fixable: true,
  level: RuleLevel.Error
});
const RuleDisableScroll = generateBasicJsonConfigCheck(
  { name: "disable-scroll", type: RuleType.JSON },
  { result: result$1, key: "disableScroll", value: true }
);

const result = createResultItem({
  name: "renderer-skyline",
  description: "\u672A\u5F00\u542F skyline \u6E32\u67D3",
  advice: `\u5C06\u914D\u7F6E\u9879 renderer \u7F6E\u4E3A "skyline"`,
  fixable: true,
  level: RuleLevel.Error
});
const RuleRendererSkyline = generateBasicJsonConfigCheck(
  { name: "renderer-skyline", type: RuleType.JSON },
  { result, key: "renderer", value: "skyline" }
);

const formatSelector = (selector) => {
  let str = "";
  selector.children.forEach((node) => {
    if (isType$1(node, "IdSelector")) {
      str += `#${node.name}`;
    } else if (isType$1(node, "TypeSelector")) {
      str += node.name;
    } else if (isType$1(node, "ClassSelector")) {
      str += `.${node.name}`;
    } else if (isType$1(node, "Combinator")) {
      str += node.name;
    } else if (isType$1(node, "AttributeSelector")) {
      let tmp = node.name.name;
      if (node.matcher)
        tmp += node.matcher;
      if (node.value)
        tmp += isType$1(node.value, "String") ? `"${node.value.value}"` : node.value.name;
      if (node.flags)
        tmp += ` ${node.flags}`;
      str += `[${tmp}]`;
    } else if (isType$1(node, "PseudoClassSelector")) ; else if (isType$1(node, "PseudoElementSelector")) ;
  });
  return str;
};
const formatSelectorList = (selectorList) => {
  return selectorList.children.toArray().flatMap((selector) => {
    if (!isType$1(selector, "Selector"))
      return [];
    return formatSelector(selector);
  }).join(", ");
};

const resultScrollViewNotFound = createResultItem({
  name: "scroll-view-not-found",
  description: "\u5F53\u524D\u9875\u9762\u672A\u4F7F\u7528 scroll-view \u7EC4\u4EF6",
  advice: "skyline \u4E0D\u652F\u6301\u9875\u9762\u5168\u5C40\u6EDA\u52A8\uFF0C\u82E5\u9875\u9762\u8D85\u8FC7\u4E00\u5C4F\uFF0C\u9700\u8981\u4F7F\u7528 scroll-view \u7EC4\u4EF6\u5B9E\u73B0\u6EDA\u52A8",
  level: RuleLevel.Warn
});
const resultScrollViewImproperType = createResultItem({
  name: "scroll-view-type",
  description: `scroll-view \u672A\u663E\u5F0F\u6307\u5B9A type \u7C7B\u578B`,
  advice: `\u5F53\u524D scroll-view \u53EA\u652F\u6301 type=list \u4E14\u9700\u663E\u5F0F\u6307\u5B9A\uFF0C\u8BE6\u89C1\u6587\u6863\uFF08\u94FE\u63A5\u5F85\u5B9A\uFF09`,
  fixable: true,
  level: RuleLevel.Error
});
const resultScrollViewOptimize = createResultItem({
  name: "scroll-view-optimize",
  description: `\u672A\u80FD\u5145\u5206\u5229\u7528 scroll-view \u6309\u9700\u6E32\u67D3\u7684\u673A\u5236`,
  advice: `scroll-view \u4F1A\u6839\u636E\u76F4\u63A5\u5B50\u8282\u70B9\u662F\u5426\u5728\u5C4F\u6765\u6309\u9700\u6E32\u67D3\uFF0C\u82E5\u53EA\u6709\u4E00\u4E2A\u76F4\u63A5\u5B50\u8282\u70B9\u5219\u6027\u80FD\u4F1A\u9000\u5316\uFF0C\u5982 <scroll-view type=list scroll-y> <view wx:for=""/> </scroll-view>`,
  level: RuleLevel.Verbose
});
const resultScrollViewXY = createResultItem({
  name: "scroll-view-x-y",
  description: `scroll-view \u6682\u4E0D\u652F\u6301\u6C34\u5E73\u5782\u76F4\u65B9\u5411\u540C\u65F6\u6EDA\u52A8`,
  advice: `skyline \u540E\u7EED\u7248\u672C\u4F1A\u652F\u6301`,
  level: RuleLevel.Info
});
const resultScrollMargin = createResultItem({
  name: "scroll-view-margin",
  description: `scroll-view \u7EC4\u4EF6\u7684\u76F4\u63A5\u5B50\u8282\u70B9 margin \u65E0\u6548`,
  advice: `\u9700\u8981\u7ED9\u8BBE\u7F6E\u4E86 margin \u7684\u76F4\u63A5\u5B50\u8282\u70B9\u5957\u591A\u4E00\u5C42 view\u3002skyline \u540E\u7EED\u7248\u672C\u8003\u8651\u4ECE\u5E03\u5C40\u7B97\u6CD5\u4E0A\u652F\u6301`,
  level: RuleLevel.Info
});
const RuleScroolViewWXML = defineRule({ name: "scroll-view-wxml", type: RuleType.WXML }, (ctx) => {
  let scrollViewCount = 0;
  ctx.lifetimes({
    before: () => {
      scrollViewCount = 0;
    },
    onVisit: (node) => {
      if (!isType(node, "Tag") || node.name !== "scroll-view")
        return;
      scrollViewCount++;
      let hasTypeList = DomUtils.getAttributeValue(node, "type") === "list";
      if (!hasTypeList) {
        const { start, end, path } = getLocationByNode(node);
        ctx.addResultWithPatch(
          {
            ...resultScrollViewImproperType,
            loc: {
              startIndex: start,
              endIndex: end,
              path
            }
          },
          {
            patchedStr: `<scroll-view type="list"`,
            loc: {
              start: node.startIndex,
              end: node.startIndex + "<scroll-view".length,
              path: path ?? ctx.env.path
            }
          }
        );
      }
      if (DomUtils.getAttributeValue(node, "scroll-x") === "true" && DomUtils.getAttributeValue(node, "scroll-y") === "true") {
        const { start, end, path } = getLocationByNode(node);
        ctx.addResult({
          ...resultScrollViewXY,
          loc: {
            startIndex: start,
            endIndex: end,
            path
          }
        });
      }
      if (hasChildren(node)) {
        const trimedChildren = node.childNodes.filter((child) => {
          if (isType(child, "Tag"))
            return true;
          if (isType(child, "Text") && child.data.trim() !== "")
            return true;
          return false;
        });
        if (trimedChildren.length === 1 && isType(trimedChildren[0], "Tag") && !Reflect.has(trimedChildren[0].attribs, "wx:for")) {
          const { start, end, path } = getLocationByNode(node);
          ctx.addResult({
            ...resultScrollViewOptimize,
            loc: {
              startIndex: start,
              endIndex: end,
              path
            }
          });
        }
      }
    },
    after: () => {
      if (scrollViewCount === 0)
        ctx.addResult(resultScrollViewNotFound);
    }
  });
});
const RuleScroolViewWXSS = defineRule({ name: "scroll-view-wxss", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node, walkCtx) => {
      if (!isType$1(node, "Declaration") || !node.property.startsWith("margin"))
        return;
      const wxmlFilename = ctx.getRelatedWXMLFilename();
      const ast = ctx.getRelatedWXMLAst();
      const prelude = walkCtx.rule?.prelude;
      if (!ast || !prelude)
        return;
      const selector = isType$1(prelude, "Raw") ? prelude.value : formatSelectorList(prelude);
      const children = selectAll(selector, ast);
      for (const child of children) {
        if (child.parent && isType(child.parent, "Tag") && child.parent.name === "scroll-view") {
          const { start, end, path } = getLocationByNode(child);
          ctx.addResult({
            ...resultScrollMargin,
            loc: {
              startIndex: child.startIndex,
              endIndex: child.endIndex,
              path: path ?? wxmlFilename ?? null
            }
          });
        }
      }
    }
  });
});
const RuleScrollView = [RuleScroolViewWXML, RuleScroolViewWXSS];

const serialize = (node) => JSON.stringify(node, null, 2);

const resolvePath = (currentPath, rootPath, filePath) => {
  let path = "";
  if (filePath.startsWith("/")) {
    path = join(rootPath, filePath);
  } else {
    path = resolve(dirname(currentPath), filePath);
  }
  return path;
};

const collectImportedWXSS = async (wxssPaths, rootPath, shouldExclude) => {
  const originPaths = wxssPaths.slice();
  const wxssSet = new Set(wxssPaths);
  const rule = defineRule({ name: "collect-imported-wxss", type: RuleType.WXSS }, (ctx) => {
    ctx.lifetimes({
      onVisit: (node) => {
        if (!ctx.env)
          return;
        if (!isType$1(node, "Atrule") || node.name !== "import" || !node.prelude || !isType$1(node.prelude, "AtrulePrelude")) {
          return;
        }
        const { path: currentPath, wxssPaths: wxssPaths2, wxssSet: wxssSet2, rootPath: rootPath2 } = ctx.env;
        node.prelude.children.forEach((child) => {
          let path = null;
          if (isType$1(child, "String")) {
            path = child.value;
          } else if (isType$1(child, "Url") && isType$1(child.value, "String")) {
            path = child.value.value;
          }
          if (!path?.endsWith(".wxss"))
            return;
          path = resolvePath(currentPath, rootPath2, path);
          if (!existsSync(path) || wxssSet2.has(path) || shouldExclude?.(path))
            return;
          wxssSet2.add(path);
          wxssPaths2.push(path);
        });
      }
    });
  });
  for (const wxssPath of wxssPaths) {
    const wxss = (await readFile(wxssPath)).toString();
    const env = { rootPath, path: wxssPath, wxssSet, wxssPaths };
    parse({ wxss, Rules: [rule], env });
  }
  for (const wxssPath of originPaths) {
    wxssSet.delete(wxssPath);
  }
  return wxssSet;
};

const MAX_CODE_FRAME_LENGTH = 1024;
const formatSourceCodeLocation = (rawStr, loc, options = {}) => {
  const { withCodeFrame = false, alternativeFilename } = options;
  let location;
  if ("startCol" in loc) {
    location = loc;
  } else {
    const finder = lineColumn(rawStr);
    const { line: startLn = -1, col: startCol = -1 } = finder.fromIndex(loc.startIndex) ?? {};
    const { line: endLn = -1, col: endCol = -1 } = finder.fromIndex(loc.endIndex) ?? {};
    location = { startLn, startCol, endLn, endCol, path: loc.path };
  }
  const filenameWithLnCol = format("%s:%d:%d", loc.path ?? alternativeFilename, location.startLn, location.startCol);
  if (!withCodeFrame)
    return filenameWithLnCol;
  const codeFrame = codeFrameColumns(
    rawStr,
    {
      start: {
        line: location.startLn,
        column: location.startCol
      },
      end: {
        line: location.endLn,
        column: location.endCol
      }
    },
    {
      linesAbove: 1,
      linesBelow: 1,
      forceColor: true
    }
  );
  if (codeFrame.length > MAX_CODE_FRAME_LENGTH)
    return filenameWithLnCol;
  return [filenameWithLnCol, codeFrame].join("\n");
};

const getUniqueKey = (path, tmplName) => `${tmplName}`;
const Rule = defineRule(
  { name: "collect-template", type: RuleType.WXML },
  (ctx) => {
    ctx.lifetimes({
      onVisit: (node, walkerContext) => {
        if (!isType(node, "Tag"))
          return;
        if (node.name === "template") {
          const { is, name } = node.attribs;
          if (is) {
            const key = getUniqueKey(ctx.env.path, is);
            const { fragment, fromFile } = ctx.env.tmplFragments.get(key) ?? {};
            if (!fragment)
              return;
            replaceChildWithChildren(node, fragment.childNodes, {
              attachFilename: fromFile ?? ctx.env.path
            });
          } else if (name) {
            const key = getUniqueKey(ctx.env.path, name);
            if (ctx.env.tmplFragments.has(key))
              return;
            ctx.env.tmplFragments.set(key, {
              fragment: node,
              fromFile: ctx.env.path
            });
            replaceChildWithChildren(node, []);
          }
        } else if (node.name === "include") {
          const { src } = node.attribs;
          if (!src)
            return;
          const srcPath = resolvePath(ctx.env.path, ctx.env.rootPath, src);
          let { fragment: srcAST, fromFile } = ctx.env.includeFragments.get(srcPath) ?? {};
          if (!srcAST)
            [srcAST] = collectTemplate([srcPath], ctx.env);
          replaceChildWithChildren(node, srcAST.childNodes, {
            attachFilename: srcPath
          });
        } else if (node.name === "import") {
          const { src } = node.attribs;
          const srcPath = resolvePath(ctx.env.path, ctx.env.rootPath, src);
          let { fragment: srcAST, fromFile } = ctx.env.importFragments.get(srcPath) ?? {};
          if (!srcAST)
            [srcAST] = collectTemplate([srcPath], ctx.env);
          replaceChildWithChildren(node, []);
        }
      }
    });
  }
);
const collectTemplate = (wxmlPaths, env) => {
  [...wxmlPaths];
  const newEnv = env ?? {
    rootPath: "",
    path: "",
    wxmlPaths,
    importFragments: /* @__PURE__ */ new Map(),
    includeFragments: /* @__PURE__ */ new Map(),
    tmplFragments: /* @__PURE__ */ new Map()
  };
  return wxmlPaths.map((path) => {
    const wxml = readFileSync(path).toString();
    let { astWXML } = parse({ wxml, Rules: [Rule], env: { ...newEnv, path } });
    return astWXML;
  });
};

const Rules = [
  RuleNagivator,
  RuleNoInlineText,
  RuleNoSvgStyleTag,
  RuleUnsupportedComponent,
  RuleDisplayFlex,
  RuleDisplayInline,
  RuleMarkWxFor,
  RulePositionFixed,
  RuleTextOverflowEllipse,
  RuleNoNativeNav,
  RuleDisableScroll,
  RuleRendererSkyline,
  RuleScrollView
].flat();
const logColor = {
  [RuleLevel.Verbose]: chalk.cyan,
  [RuleLevel.Info]: chalk.blue,
  [RuleLevel.Warn]: chalk.yellow,
  [RuleLevel.Error]: chalk.red
};
const splitString = (input) => {
  if (Array.isArray(input))
    return input;
  return input.split(",").map((item) => item.trim());
};
const cli = new Command();
cli.name(pkg.name);
cli.version(pkg.version);
cli.option(
  "-p, --path [string]",
  "\u5DE5\u7A0B\u7684\u6839\u76EE\u5F55",
  (input) => path.resolve(input),
  ""
);
cli.option(
  "-l, --log-level [number]",
  "\u4F9D\u65E5\u5FD7\u7B49\u7EA7\u8FC7\u6EE4\uFF0C\u4ECE 0 \u5230 3",
  parseInt,
  0
);
cli.option(
  "-i, --ignore [string]",
  "\u8981\u5FFD\u7565\u7684\u89C4\u5219\u540D\uFF0C\u7528\u534A\u89D2\u9017\u53F7\u5206\u9694",
  splitString,
  []
);
cli.option(
  "-e, --exclude [string]",
  "\u8981\u6392\u9664\u7684\u8DEF\u5F84\u540D\u7684\u6B63\u5219\u8868\u8FBE\u5F0F\uFF0C\u7528\u534A\u89D2\u9017\u53F7\u5206\u9694",
  splitString,
  []
);
cli.parse(argv);
const options = cli.opts();
const main = async () => {
  let appJsonPath = "";
  let appJsonObject = null;
  let pageJsonObjects = [];
  const disabledRules = new Set(options.ignore);
  const excludedFiles = options.exclude.map((str) => new RegExp(str));
  const isPathExcluded = (path2) => excludedFiles.some((regex) => regex.test(path2));
  const getAppJsonFromPath = async (path2) => {
    try {
      appJsonPath = resolve(path2, "app.json");
      const appJsonFile = await readFile(appJsonPath);
      appJsonObject = JSON.parse(appJsonFile.toString());
    } catch (e) {
      throw "\u65E0\u6548 app.json\uFF0C\u8BF7\u68C0\u67E5\u8DEF\u5F84\u548C\u8BED\u6CD5\u662F\u5426\u6B63\u786E";
    }
  };
  if (options.path) {
    await getAppJsonFromPath(options.path);
  }
  const pages = [];
  const validatePath = async (input) => {
    await getAppJsonFromPath(input);
    const subPackages = appJsonObject["subpackages"] ?? appJsonObject["subPackages"] ?? [];
    pages.push(...appJsonObject["pages"] ?? []);
    for (const subPackage of subPackages) {
      const { root, pages: subPackagePages } = subPackage;
      pages.push(...subPackagePages.map((page) => join(root, page)));
    }
    for (const page of pages) {
      const pageJsonPath = resolve(input, page + ".json");
      try {
        const pageJsonFile = await readFile(pageJsonPath);
        const pageJsonObject = JSON.parse(pageJsonFile.toString());
        pageJsonObjects[page] = pageJsonObject;
      } catch (err) {
        throw `\u9875\u9762 ${page} \u7684\u914D\u7F6E\u6587\u4EF6\u4E0D\u5B58\u5728`;
      }
    }
  };
  if (options.path) {
    await validatePath(options.path);
  } else {
    await inquirer.prompt({
      type: "input",
      name: "path",
      message: "\u8BF7\u8F93\u5165\u5DE5\u7A0B\u7684\u6839\u76EE\u5F55:",
      default: cwd(),
      when: !options.path,
      validate: async (input) => {
        await validatePath(input);
        return true;
      },
      filter: (input) => resolve(input)
    }).then((answer) => {
      if (answer.path) {
        options.path = answer.path;
      }
    });
  }
  let globalSkyline = appJsonObject["renderer"] === "skyline";
  const answers = await inquirer.prompt([
    {
      type: "confirm",
      name: "appJsonEnableDynamicInjection",
      message: `skyline \u4F9D\u8D56\u6309\u9700\u6CE8\u5165\u7279\u6027\uFF0C\u7ACB\u5373\u5F00\u542F\uFF1F
\u{1F4A1} \u6309\u9700\u6CE8\u5165\u7279\u6027\u8BE6\u89C1\u6587\u6863 https://developers.weixin.qq.com/miniprogram/dev/framework/ability/lazyload.html`,
      default: false,
      when: (hash) => {
        const flag = appJsonObject["lazyCodeLoading"] !== "requiredComponents";
        if (!flag)
          stdout.write(chalk.green("\u2705 skyline \u4F9D\u8D56\u6309\u9700\u6CE8\u5165\u7279\u6027\uFF0C\u5DF2\u5F00\u542F\n"));
        return flag;
      }
    },
    {
      type: "confirm",
      name: "globalSkyline",
      message: `\u662F\u5426\u5168\u5C40\u5F00\u542F skyline?
\u{1F4A1} \u5168\u5C40\u5F00\u542F skyline \u610F\u5473\u7740\u6574\u4E2A\u5C0F\u7A0B\u5E8F\u9700\u8981\u9002\u914D skyline\uFF0C\u5EFA\u8BAE\u5B58\u91CF\u5DE5\u7A0B\u9010\u4E2A\u9875\u9762\u5F00\u542F\uFF0C\u5168\u65B0\u5DE5\u7A0B\u53EF\u5168\u5C40\u5F00\u542F`,
      default: false,
      when: (hash) => {
        const flag = !globalSkyline;
        if (!flag)
          stdout.write(chalk.green("\u2705 \u5DF2\u5168\u5C40\u5F00\u542F skyline\n"));
        return flag;
      }
    },
    {
      type: "input",
      name: "skylinePages",
      message: "\u8BF7\u8F93\u5165\u9700\u8981\u8FC1\u79FB\u7684\u9875\u9762\uFF08\u7528\u534A\u89D2\u9017\u53F7\u5206\u9694\uFF09",
      filter: (input) => {
        if (Array.isArray(input))
          return input;
        return input.split(",").map((page) => page.trim());
      },
      validate: (pages2) => {
        for (const page of pages2) {
          if (!pageJsonObjects[page])
            return `\u9875\u9762 ${page} \u4E0D\u5B58\u5728`;
        }
        return true;
      },
      default: () => Object.entries(pageJsonObjects).filter(([k, v]) => v["renderer"] === "skyline").map(([k]) => k),
      when: () => appJsonObject["renderer"] !== "skyline"
    }
  ]);
  if (!existsSync(options.path))
    return;
  if (!appJsonObject)
    return;
  if (answers.globalSkyline)
    globalSkyline = answers.globalSkyline;
  if (answers.appJsonEnableDynamicInjection) {
    appJsonObject["lazyCodeLoading"] = "requiredComponents";
  }
  if (globalSkyline) {
    appJsonObject["renderer"] = "skyline";
    answers.skylinePages = Object.keys(pageJsonObjects);
  }
  writeFile(appJsonPath, serialize(appJsonObject));
  if (appJsonObject.useExtendedLib) {
    stdout.write(
      format(
        chalk.bold("\n============ %s %s ============\n"),
        "App",
        chalk.blue("app.json")
      )
    );
    stdout.write(
      format(
        logColor[RuleLevel.Error]("@%s %s"),
        "useExtendedLib",
        "app.json \u6682\u4E0D\u652F\u6301 useExtendedLib"
      )
    );
    stdout.write(
      format("\n\u{1F4A1} %s\n", chalk.gray("\u5B8C\u6574\u529F\u80FD skyline \u540E\u7EED\u7248\u672C\u4F1A\u652F\u6301"))
    );
    stdout.write(format("  %s\n", appJsonPath));
  }
  const scan = async () => {
    const checkList = [];
    const fileMap = /* @__PURE__ */ new Map();
    for (const page of answers.skylinePages) {
      const path2 = resolve(options.path, page);
      if (isPathExcluded(path2))
        continue;
      checkList.push(path2);
      fileMap.set(path2, "page");
    }
    const dfs = async (base, obj, isDir = false) => {
      let pathDirname = base;
      if (!isDir) {
        if (base.startsWith(options.path)) {
          pathDirname = dirname(base);
        } else {
          pathDirname = dirname(join("./", base));
        }
      }
      const compList = Object.values(obj?.["usingComponents"] ?? {});
      for (const comp of compList) {
        let path2 = comp.startsWith("/") ? join(options.path, comp) : resolve(pathDirname, comp);
        try {
          const stat = lstatSync(path2);
          if (stat.isDirectory())
            path2 = resolve(path2, "index");
        } catch (e) {
        }
        if (fileMap.has(path2) || isPathExcluded(path2) || !existsSync(`${path2}.json`))
          continue;
        checkList.push(path2);
        fileMap.set(path2, "comp");
        const json = JSON.parse((await readFile(`${path2}.json`)).toString());
        await dfs(path2, json);
      }
    };
    await dfs(options.path, appJsonObject, true);
    for (const page of answers.skylinePages) {
      const pagePath = resolve(options.path, page);
      pageJsonObjects[page] && await dfs(pagePath, pageJsonObjects[page]);
    }
    const wxssFiles = [];
    for (const pageOrComp of checkList) {
      wxssFiles.push(...await globby([`${pageOrComp}.wxss`]));
    }
    const importedWXSS = await collectImportedWXSS(
      wxssFiles,
      options.path,
      isPathExcluded
    );
    const stringPatches = [];
    let fileCount = 0;
    let resultCount = 0;
    const runOnFile = async (filename, env = {}) => {
      let wxss = "";
      let wxml = "";
      let json = "";
      let astWXML;
      let astWXSS;
      let astJSON;
      fileCount++;
      if (!existsSync(filename))
        return [];
      const raw = (await readFile(filename)).toString();
      if (filename.endsWith("wxss")) {
        wxss = raw;
      } else if (filename.endsWith("wxml")) {
        wxml = raw;
        astWXML = collectTemplate([filename])[0];
      } else if (filename.endsWith("json")) {
        json = raw;
      }
      let parsed = parse({
        wxml,
        wxss,
        json,
        astWXML,
        astWXSS,
        astJSON,
        Rules,
        env: { ...env, path: filename }
      });
      const resultItems = [];
      for (const { patches, results } of parsed.ruleResults) {
        for (const item of results) {
          if (disabledRules.has(item.name))
            continue;
          resultItems.push({
            filename,
            ...item
          });
        }
        stringPatches.push(
          ...patches.filter((patch) => !disabledRules.has(patch.name))
        );
      }
      return resultItems;
    };
    const sortResults = (resultItems) => resultItems.sort((a, b) => {
      return a.level !== b.level ? b.level - a.level : a.name.localeCompare(b.name);
    });
    const printResults = (resultItems) => {
      resultCount += resultItems.length;
      let lastName = null;
      for (const result of resultItems) {
        if (options.logLevel > result.level)
          continue;
        const {
          loc,
          advice,
          description,
          name,
          level,
          fixable,
          filename,
          withCodeFrame
        } = result;
        const color = logColor[level];
        let filePath = "";
        const rawStr = readFileSync(loc?.path ?? result.filename).toString();
        if (!loc) {
          filePath = filename;
        } else {
          filePath = formatSourceCodeLocation(rawStr, loc, {
            withCodeFrame,
            alternativeFilename: filename
          });
        }
        if (lastName !== name) {
          stdout.write("\n");
          stdout.write(format(color("@%s %s"), name, description));
          fixable && stdout.write(chalk.green(" [\u53EF\u81EA\u52A8\u5B8C\u6210]"));
          advice && stdout.write(format("\n\u{1F4A1} %s\n", chalk.gray(advice)));
        }
        stdout.write(format("  %s\n", filePath));
        lastName = name;
      }
    };
    for (const pageOrComp of checkList) {
      const type = fileMap.get(pageOrComp);
      const files = ["json", "wxml", "wxss"].map((ext) => [pageOrComp, ext].join(".")).filter((file) => existsSync(file));
      const astMap = /* @__PURE__ */ new Map();
      let results = [];
      for (const filename of files) {
        const result = await runOnFile(filename, { astMap });
        results.push(...result);
      }
      if (results.length) {
        stdout.write(
          format(
            chalk.bold("\n============ %s %s ============\n"),
            type?.toUpperCase(),
            chalk.blue(relative(options.path, pageOrComp))
          )
        );
        printResults(sortResults(results));
      }
    }
    {
      const jobs = [...importedWXSS].map((filename) => runOnFile(filename));
      const results = (await Promise.all(jobs)).flat();
      if (results.length) {
        stdout.write(
          format(chalk.bold("\n============ %s ============\n"), "Imported")
        );
        printResults(sortResults(results));
      }
    }
    stdout.write("\n");
    const fixMessage = format(
      "%d \u4E2A\u6587\u4EF6\u4E2D\u5171\u6709 %d \u5904\u95EE\u9898\uFF0C\u5176\u4E2D %d \u5904\u53EF\u4EE5\u81EA\u52A8\u4FEE\u590D\uFF0C\u662F\u5426\u8FDB\u884C\uFF1F\n",
      fileCount,
      resultCount,
      stringPatches.length
    );
    const fixAnswer = await inquirer.prompt([
      {
        type: "confirm",
        name: "applyFix",
        message: fixMessage,
        default: false,
        when: stringPatches.length > 0
      }
    ]);
    if (fixAnswer.applyFix) {
      const filePatchMap = /* @__PURE__ */ new Map();
      for (const patch of stringPatches) {
        const { path: path2 } = patch.loc;
        if (!filePatchMap.has(path2)) {
          if (!existsSync(path2))
            continue;
          filePatchMap.set(path2, {
            content: (await readFile(path2)).toString(),
            patches: []
          });
        }
        filePatchMap.get(path2)?.patches.push(patch);
      }
      for (const [path2, { patches, content }] of filePatchMap) {
        const patchedString = applyPatchesOnString(content, patches);
        await writeFile(path2, patchedString.toString());
      }
      stdout.write(chalk.green("\u2705 \u4FEE\u590D\u5B8C\u6210"));
    }
    const { again } = await inquirer.prompt([
      {
        type: "confirm",
        name: "again",
        message: "\u662F\u5426\u91CD\u65B0\u626B\u63CF\uFF1F",
        default: false
      }
    ]);
    if (again)
      await scan();
  };
  await scan();
};
main().catch((err) => {
  console.error(chalk.blue("\u274C"), err.message, err.stack);
});

export { main as default };
