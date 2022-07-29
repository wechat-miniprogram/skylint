#!/usr/bin/env node
import { Command } from "commander";
import { cwd, argv, stdout } from "process";
import { globby } from "globby";
import { readFile, writeFile } from "fs/promises";
import chalk from "chalk";
import pkg from "../package.json";
import { BasicParseEnv, parse } from "./parser";

// WXML rules
import RuleNagivator from "./rules/navigator";
import RuleNoInlineText from "./rules/no-inline-text";
import RuleNoSvgStyleTag from "./rules/no-svg-style-tag";
import RuleUnsupportedComponent from "./rules/unsupported-component";
// WXSS rules
import RuleBoxSizing from "./rules/box-sizing";
import RuleDarkMode from "./rules/darkmode";
import RuleDisplayFlex from "./rules/display-flex";
import RuleDisplayInline from "./rules/display-inline";
import RuleMarkWxFor from "./rules/mark-wx-for";
import RuleNoCalc from "./rules/no-calc";
import RuleNoCSSAnimation from "./rules/no-css-animation";
import RuleNoPseudo from "./rules/no-pseudo";
import RulePositionFixed from "./rules/position-fixed";
import RuleTextOverflowEllipse from "./rules/text-overflow-ellipse";
// JSON rules
import RuleNoNativeNav from "./rules/no-native-nav";
import RuleDisableScroll from "./rules/disable-scroll";
import RuleRendererSkyline from "./rules/renderer-skyline";
// Mixed rules
import RuleScrollView from "./rules/scroll-view";
import RuleWeuiExtendedlib from "./rules/weui-extendedlib";

import { RuleLevel, RuleResultItem } from "./rules/interface";
import { format } from "util";
// import { serialize as serializeHTML } from "./serializer/html";
// import { serialize as serializeCSS } from "./serializer/css";
import { serialize as serializeJSON } from "./serializer/json";

import inquirer from "inquirer";
import path, { resolve, dirname, relative, join } from "path";
import { Patch, applyPatchesOnString } from "./patch";
import { existsSync, readFileSync, lstatSync } from "fs";
import { collectImportedWXSS } from "./utils/collect-wxss";
import { formatSourceCodeLocation } from "./utils/print-code";
import { Document } from "domhandler";
import { NodeTypeMap } from "./walker/html";
import { Node as CssNode } from "./walker/css";
import { Node as JsonNode, ValueNode } from "./walker/json";
import { collectTemplate } from "./utils/collect-template";

const Rules = [
  // WXML rules
  RuleNagivator,
  RuleNoInlineText,
  RuleNoSvgStyleTag,
  RuleUnsupportedComponent,
  // WXSS rules
  RuleBoxSizing,
  RuleDarkMode,
  RuleDisplayFlex,
  RuleDisplayInline,
  RuleMarkWxFor,
  RuleNoCalc,
  RuleNoCSSAnimation,
  RuleNoPseudo,
  RulePositionFixed,
  RuleTextOverflowEllipse,
  // JSON rules
  RuleNoNativeNav,
  RuleDisableScroll,
  RuleRendererSkyline,
  // Mixed rules
  RuleScrollView,
  RuleWeuiExtendedlib,
].flat();

const logColor = {
  [RuleLevel.Verbose]: chalk.cyan,
  [RuleLevel.Info]: chalk.blue,
  [RuleLevel.Warn]: chalk.yellow,
  [RuleLevel.Error]: chalk.red,
};

interface ICliOptions {
  path?: string;
  logLevel: number;
  ignore: string[];
  exclude: string[];
}

const splitString = (input: string | string[]) => {
  if (Array.isArray(input)) return input;
  return input.split(",").map((item) => item.trim());
};

const cli = new Command();
cli.name(pkg.name);
cli.version(pkg.version);

cli.option("-p, --path [string]", "工程的根目录", resolve);
cli.option("-l, --log-level [number]", "依日志等级过滤，从 0 到 3", parseInt, 0);
cli.option("-i, --ignore [string]", "要忽略的规则名，用半角逗号分隔", splitString, [] as string[]);
cli.option("-e, --exclude [string]", "要排除的路径名的正则表达式，用半角逗号分隔", splitString, [] as string[]);

cli.parse(argv);

const options = cli.opts<ICliOptions>();

interface PromptAnswer {
  autoAppJson: boolean;
  appJsonEnableDynamicInjection: boolean;
  globalSkyline: boolean;
  usePageSelector: boolean;
  skylinePages: string[];
}

interface ExtendedRuleResultItem extends RuleResultItem {
  filename: string;
}

const main = async () => {
  let appJsonPath: string = "";
  let appJsonObject: any = null;
  let pageJsonObjects: Record<string, any> = [];

  const disabledRules = new Set(options.ignore);
  const excludedFiles = options.exclude.map((str) => new RegExp(str));
  const isPathExcluded = (path: string) => excludedFiles.some((regex) => regex.test(path));

  const getAppJsonFromPath = async (path: string) => {
    try {
      appJsonPath = resolve(path, "app.json");
      const appJsonFile = await readFile(appJsonPath);
      appJsonObject = JSON.parse(appJsonFile.toString());
    } catch (e) {
      return "无效 app.json，请检查路径和语法是否正确";
    }
  };

  if (options.path && existsSync(options.path)) {
    if (!(await getAppJsonFromPath(options.path))) return;
  }

  const pages: string[] = [];

  await inquirer
    .prompt<Record<"path", string>>({
      type: "input",
      name: "path",
      message: "请输入工程的根目录:",
      default: cwd(),
      when: !options.path,
      validate: async (input) => {
        const err = await getAppJsonFromPath(input);
        if (err) return err;
        const subPackages = appJsonObject["subPackages"] ?? [];
        pages.push(...(appJsonObject["pages"] ?? []));
        for (const subPackage of subPackages) {
          const { root, pages: subPackagePages } = subPackage;
          pages.push(...subPackagePages.map((page: string) => join(root, page)));
        }

        for (const page of pages) {
          const pageJsonPath = resolve(input, page + ".json");
          try {
            const pageJsonFile = await readFile(pageJsonPath);
            const pageJsonObject = JSON.parse(pageJsonFile.toString());
            pageJsonObjects[page] = pageJsonObject;
          } catch (err) {
            return `页面 ${page} 的配置文件不存在`;
          }
        }

        return true;
      },
      filter: (input) => resolve(input),
    })
    .then((answer) => {
      if (answer.path) {
        options.path = answer.path;
      }
    });

  let globalSkyline = appJsonObject["renderer"] === "skyline";

  const answers = await inquirer.prompt<PromptAnswer>([
    {
      type: "confirm",
      name: "appJsonEnableDynamicInjection",
      message: `skyline 依赖按需注入特性，立即开启？
💡 按需注入特性详见文档 https://developers.weixin.qq.com/miniprogram/dev/framework/ability/lazyload.html`,
      default: false,
      when: (hash) => {
        const flag = appJsonObject["lazyCodeLoading"] !== "requiredComponents";
        if (!flag) stdout.write(chalk.green("✅ skyline 依赖按需注入特性，已开启\n"));
        return flag;
      },
    },
    {
      type: "confirm",
      name: "globalSkyline",
      message: `是否全局开启 skyline?
💡 全局开启 skyline 意味着整个小程序需要适配 skyline，建议存量工程逐个页面开启，全新工程可全局开启`,
      default: false,
      when: (hash) => {
        const flag = !globalSkyline;
        if (!flag) stdout.write(chalk.green("✅ 已全局开启 skyline\n"));
        return flag;
      },
    },
    {
      type: "input",
      name: "skylinePages",
      message: "请输入需要迁移的页面（用半角逗号分隔）",
      filter: (input: string | string[]) => {
        if (Array.isArray(input)) return input;
        return input.split(",").map((page) => page.trim());
      },
      validate: (pages: string[]) => {
        for (const page of pages) {
          if (!pageJsonObjects[page]) return `页面 ${page} 不存在`;
        }
        return true;
      },

      default: () =>
        Object.entries(pageJsonObjects)
          .filter(([k, v]) => v["renderer"] === "skyline")
          .map(([k]) => k),
      when: () => appJsonObject["renderer"] !== "skyline",
    },
  ]);

  if (!existsSync(options.path!)) return;

  if (!appJsonObject) return;

  if (answers.globalSkyline) globalSkyline = answers.globalSkyline;

  if (answers.appJsonEnableDynamicInjection) {
    appJsonObject["lazyCodeLoading"] = "requiredComponents";
  }

  if (globalSkyline) {
    appJsonObject["renderer"] = "skyline";
    answers.skylinePages = Object.keys(pageJsonObjects);
  }

  writeFile(appJsonPath, serializeJSON(appJsonObject));

  const scan = async () => {
    const checkList: string[] = [];

    type FileType = "page" | "comp" | "imported";

    const fileMap = new Map<string, FileType>();

    // collect pages
    // const pages: string[] = answers.skylinePages.map((page) => resolve(options.path!, page));
    for (const page of answers.skylinePages) {
      const path = resolve(options.path!, page);
      if (isPathExcluded(path)) continue;
      checkList.push(path);
      fileMap.set(path, "page");
    }
    // collect used components
    // const usedComponents: string[] = [];
    const dfs = async (base: string, obj: any, isDir = false) => {
      let pathDirname = base;
      if (!isDir) {
        if (base.startsWith(options.path!)) {
          pathDirname = dirname(base);
        } else {
          pathDirname = dirname(join("./", base));
        }
      }

      const compList: string[] = Object.values(obj?.["usingComponents"] ?? {});
      for (const comp of compList) {
        let path = comp.startsWith("/") ? join(options.path!, comp) : resolve(pathDirname, comp);
        try {
          const stat = lstatSync(path);
          if (stat.isDirectory()) path = resolve(path, "index");
        } catch (e) {}
        if (fileMap.has(path) || isPathExcluded(path) || !existsSync(`${path}.json`)) continue;
        checkList.push(path);
        fileMap.set(path, "comp");
        const json = JSON.parse((await readFile(`${path}.json`)).toString());
        await dfs(path, json);
      }
    };
    await dfs(options.path!, appJsonObject, true);
    for (const page of answers.skylinePages) {
      const pagePath = resolve(options.path!, page);
      pageJsonObjects[page] && (await dfs(pagePath, pageJsonObjects[page]));
    }

    // collect imported wxss
    const wxssFiles: string[] = [];
    for (const pageOrComp of checkList) {
      // wxssFiles.push(`${pageOrComp}.wxss`);
      wxssFiles.push(...(await globby([`${pageOrComp}.wxss`])));
    }
    const importedWXSS = await collectImportedWXSS(wxssFiles, options.path!, isPathExcluded);
    console.log(checkList);

    // collet patches
    // const stringPatchesMap = new Map<string, { raw: string; patches: Patch[] }>();
    const stringPatches: Patch[] = [];

    let fileCount = 0;
    let resultCount = 0;

    const runOnFile = async (filename: string, env: Partial<BasicParseEnv> = {}) => {
      let wxss = "";
      let wxml = "";
      let json = "";
      let astWXML: NodeTypeMap["Root"] | undefined;
      let astWXSS: CssNode | undefined;
      let astJSON: ValueNode | undefined;
      fileCount++;
      if (!existsSync(filename)) return [];
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
        env: { ...env, path: filename },
      });
      const resultItems: ExtendedRuleResultItem[] = [];
      for (const { patches, results } of parsed.ruleResults) {
        for (const item of results) {
          if (disabledRules.has(item.name)) continue;
          resultItems.push({
            filename,
            ...item,
          });
        }
        stringPatches.push(...patches.filter((patch) => !disabledRules.has(patch.name)));
      }
      return resultItems;
    };

    const sortResults = (resultItems: ExtendedRuleResultItem[]) =>
      resultItems.sort((a, b) => {
        return a.level !== b.level ? b.level - a.level : a.name.localeCompare(b.name);
      });

    const printResults = (resultItems: ExtendedRuleResultItem[]) => {
      resultCount += resultItems.length;
      let lastName: string | null = null;
      for (const result of resultItems) {
        if (options.logLevel > result.level) continue;
        const { loc, advice, description, name, level, fixable, filename, withCodeFrame } = result;
        const color = logColor[level];

        let filePath = "";
        const rawStr = readFileSync(loc?.path ?? result.filename).toString();
        if (!loc) {
          filePath = filename;
        } else {
          filePath = formatSourceCodeLocation(rawStr, loc, {
            withCodeFrame,
            alternativeFilename: filename,
          });
        }
        if (lastName !== name) {
          stdout.write("\n");
          stdout.write(format(color("@%s %s"), name, description));
          fixable && stdout.write(chalk.green(" [可自动完成]"));
          advice && stdout.write(format("\n💡 %s\n", chalk.gray(advice)));
        }
        stdout.write(format("  %s\n", filePath));
        lastName = name;
      }
    };

    for (const pageOrComp of checkList) {
      const type = fileMap.get(pageOrComp);
      const files = ["json", "wxml", "wxss"]
        .map((ext) => [pageOrComp, ext].join("."))
        .filter((file) => existsSync(file));
      const astMap = new Map();
      let results: ExtendedRuleResultItem[] = [];
      for (const filename of files) {
        const result = await runOnFile(filename, { astMap });
        results.push(...result);
      }
      if (results.length) {
        stdout.write(
          format(
            chalk.bold("\n============ %s %s ============\n"),
            type?.toUpperCase(),
            chalk.blue(relative(options.path!, pageOrComp))
          )
        );
        printResults(sortResults(results));
      }
    }

    {
      const jobs = [...importedWXSS].map((filename) => runOnFile(filename));
      const results = (await Promise.all(jobs)).flat();
      if (results.length) {
        stdout.write(format(chalk.bold("\n============ %s ============\n"), "Imported"));
        printResults(sortResults(results));
      }
    }

    stdout.write("\n");
    const fixMessage = format(
      "%d 个文件中共有 %d 处问题，其中 %d 处可以自动修复，是否进行？\n",
      fileCount,
      resultCount,
      stringPatches.length
    );

    type FixAnswer = Record<"applyFix", boolean>;

    const fixAnswer = await inquirer.prompt<FixAnswer>([
      {
        type: "confirm",
        name: "applyFix",
        message: fixMessage,
        default: false,
        when: stringPatches.length > 0,
      },
    ]);

    if (fixAnswer.applyFix) {
      const filePatchMap = new Map<string, { content: string; patches: Patch[] }>();
      for (const patch of stringPatches) {
        const { path } = patch.loc;
        if (!filePatchMap.has(path)) {
          if (!existsSync(path)) continue;
          filePatchMap.set(path, {
            content: (await readFile(path)).toString(),
            patches: [],
          });
        }
        filePatchMap.get(path)?.patches.push(patch);
      }
      for (const [path, { patches, content }] of filePatchMap) {
        const patchedString = applyPatchesOnString(content, patches);
        await writeFile(path, patchedString.toString());
      }
      stdout.write(chalk.green("✅ 修复完成"));
    }

    type AgainAnswer = Record<"again", boolean>;

    const { again } = await inquirer.prompt<AgainAnswer>([
      {
        type: "confirm",
        name: "again",
        message: "是否重新扫描？",
        default: false,
      },
    ]);

    if (again) await scan();
  };
  await scan();
};

main().catch((err: Error) => {
  console.error(chalk.blue("❌"), err.message, err.stack);
});

export default main;
