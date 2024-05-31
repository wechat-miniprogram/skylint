import { walk as walk$3, parse as parse$1 } from 'css-tree';
import { parseDocument } from 'htmlparser2';
import parseJSON from 'json-to-ast';
import MagicString from 'magic-string';
import { hasChildren } from 'domhandler';

var PatchStatus = /* @__PURE__ */ ((PatchStatus2) => {
  PatchStatus2[PatchStatus2["Pending"] = 0] = "Pending";
  PatchStatus2[PatchStatus2["Applied"] = 1] = "Applied";
  PatchStatus2[PatchStatus2["Failed"] = 2] = "Failed";
  return PatchStatus2;
})(PatchStatus || {});
const sortPatchesByLoc = (patches) => patches.sort((a, b) => {
  return a.loc.start - b.loc.start;
});
const applyPatchesOnString = (rawString, patches) => {
  const str = new MagicString(rawString);
  const sortedPatches = sortPatchesByLoc(patches);
  const nonOverLappedPatches = sortedPatches;
  const len = nonOverLappedPatches.length;
  for (let i = 0; i < len; i++) {
    const { loc, patchedStr } = sortedPatches[i];
    const range = loc.end - loc.start;
    if (range === 0) {
      str.appendRight(loc.start, patchedStr);
    } else if (range > 0) {
      str.overwrite(loc.start, loc.end, patchedStr);
    }
  }
  return str;
};

var RuleLevel = /* @__PURE__ */ ((RuleLevel2) => {
  RuleLevel2[RuleLevel2["Verbose"] = 0] = "Verbose";
  RuleLevel2[RuleLevel2["Info"] = 1] = "Info";
  RuleLevel2[RuleLevel2["Warn"] = 2] = "Warn";
  RuleLevel2[RuleLevel2["Error"] = 3] = "Error";
  return RuleLevel2;
})(RuleLevel || {});
var RuleType = /* @__PURE__ */ ((RuleType2) => {
  RuleType2[RuleType2["Unknown"] = 0] = "Unknown";
  RuleType2[RuleType2["WXML"] = 1] = "WXML";
  RuleType2[RuleType2["WXSS"] = 2] = "WXSS";
  RuleType2[RuleType2["Node"] = 3] = "Node";
  RuleType2[RuleType2["JSON"] = 4] = "JSON";
  return RuleType2;
})(RuleType || {});
const defineRule = (info, init) => (env) => {
  const { name, type } = info;
  let hooks = {};
  let results = [];
  let patches = [];
  let astPatches = [];
  const lifetimes = (newHooks) => {
    hooks = { ...hooks, ...newHooks };
  };
  const addResult = (...newResults) => {
    results.push(...newResults);
  };
  const addResultWithPatch = (result, patch) => {
    const { name: name2 } = result;
    results.push(result);
    patches.push({
      ...patch,
      name: name2,
      status: PatchStatus.Pending
    });
  };
  const addASTPatch = (...newPatches) => astPatches.push(...newPatches);
  const addPatch = (...newPatches) => patches.push(
    ...newPatches.map((patch) => ({
      ...patch,
      status: PatchStatus.Pending
    }))
  );
  const getRelatedWXMLFilename = () => env.path.replace(/(?!\.)(wxml|json|wxss)$/, "wxml");
  const getRelatedWXMLAst = () => {
    if (!env.astMap)
      return null;
    return env.astMap.get(getRelatedWXMLFilename()) ?? null;
  };
  init({
    lifetimes,
    addASTPatch,
    addPatch,
    addResult,
    addResultWithPatch,
    getRelatedWXMLFilename,
    getRelatedWXMLAst,
    env
  });
  return {
    name,
    type,
    get results() {
      return results;
    },
    get astPatches() {
      return astPatches;
    },
    get patches() {
      return patches;
    },
    clear() {
    },
    ...hooks
  };
};
const createResultItem = (params) => {
  return {
    level: 2 /* Warn */,
    ...params
  };
};

const elementType = {
  Root: "root",
  Text: "text",
  Directive: "directive",
  Comment: "comment",
  Script: "script",
  Style: "style",
  Tag: "tag",
  CDATA: "cdata",
  Doctype: "doctype"
};
const isType$2 = (node, type) => {
  return node.type === elementType[type];
};
const walk$2 = (node, callback, ctx = { parent: node }) => {
  if (!ctx)
    ctx = { parent: node };
  if (callback(node, ctx) === false) {
    return false;
  } else {
    if (hasChildren(node)) {
      for (const childNode of node.childNodes ?? []) {
        const ret = walk$2(childNode, callback, { ...ctx, parent: node });
        if (ret === false)
          return ret;
      }
    }
    return true;
  }
};

const isType$1 = (node, type) => {
  return node.type === type;
};
const walk$1 = (node, callback) => {
  return walk$3(node, function(node2) {
    const self = this;
    callback(node2, self);
  });
};

const isType = (node, type) => {
  return node.type === type;
};
const walk = (node, callback, ctx = { parent: node }) => {
  if (!ctx)
    ctx = { parent: node };
  if (callback(node, ctx) === false) {
    return false;
  } else {
    const newCtx = { parent: node };
    if ("children" in node) {
      for (const childNode of node.children ?? []) {
        if (walk(childNode, callback, { ...newCtx }) === false)
          return false;
      }
    } else if (isType(node, "Property")) {
      if (walk(node.key, callback, { ...newCtx }) === false)
        return false;
      if (walk(node.value, callback, { ...newCtx }) === false)
        return false;
    }
    return true;
  }
};

const classifyRules = (rules) => {
  const wxmlRules = [];
  const wxssRules = [];
  const nodeRules = [];
  const jsonRules = [];
  const anyRules = [];
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
    }
  }
  return { wxmlRules, wxssRules, nodeRules, jsonRules, anyRules };
};
const runLifetimeHooks = (rules, ast, walker) => {
  rules.forEach((rule) => rule.before?.());
  walker(
    ast,
    (...args) => {
      rules.forEach((rule) => {
        rule.onVisit?.(...args);
      });
    },
    null
  );
  rules.forEach((rule) => rule.after?.());
};
const extractResultFromRules = (rules) => {
  return rules.flatMap((rule) => {
    const { name, level, results, patches } = rule;
    if (!results.length)
      return [];
    return { name, level, results, patches };
  });
};
const parse = (options) => {
  const { wxml, wxss, json, Rules = [], env } = options;
  const rules = Rules.map((Rule2) => Rule2(env));
  const { wxmlRules, wxssRules, nodeRules, jsonRules, anyRules } = classifyRules(rules);
  let { astJSON, astWXML, astWXSS } = options;
  if (wxml && !astWXML)
    astWXML = parseDocument(wxml, { xmlMode: true, withStartIndices: true, withEndIndices: true });
  if (astWXML) {
    env.astMap?.set(env.path, astWXML);
    runLifetimeHooks(wxmlRules, astWXML, walk$2);
  }
  if (wxss && !astWXSS)
    astWXSS = parse$1(wxss, { positions: true });
  if (astWXSS) {
    env.astMap?.set(env.path, astWXSS);
    runLifetimeHooks(wxssRules, astWXSS, walk$1);
  }
  if (json && !astJSON)
    astJSON = parseJSON(json);
  if (astJSON) {
    env.astMap?.set(env.path, astJSON);
    runLifetimeHooks(jsonRules, astJSON, walk);
  }
  return { astWXML, astWXSS, astJSON, ruleResults: extractResultFromRules(anyRules) };
};

export { RuleLevel as R, RuleType as a, isType$1 as b, createResultItem as c, defineRule as d, isType as e, applyPatchesOnString as f, isType$2 as i, parse as p };
