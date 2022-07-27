import { Command } from "commander";
import { cwd, argv, chdir, stdout } from "process";
import { globby } from "globby";
import { readFile } from "fs/promises";
import chalk from "chalk";
import pkg from "../package.json";
import { parse } from "./parser";

import ruleBoxSizing from "./rules/box-sizing";
import ruleDisplayFlex from "./rules/display-flex";
import ruleDisplayInline from "./rules/display-inline";
import ruleDisplayInlineBlock from "./rules/display-inline-block";
import ruleNoCalc from "./rules/no-calc";
import ruleNoPseudo from "./rules/no-pseudo";
import rulePositionFixed from "./rules/position-fixed";
import ruleScrollView from "./rules/scroll-view";
import { RuleLevel } from "./rules/interface";
import { format } from "util";

interface ICliOptions {
  path: string;
  logLevel: number;
}

const rules = [
  ruleBoxSizing,
  ruleDisplayFlex,
  ruleDisplayInline,
  ruleDisplayInlineBlock,
  ruleNoCalc,
  ruleNoPseudo,
  rulePositionFixed,
  ruleScrollView,
];

const logColor = {
  [RuleLevel.Verbose]: (str?: string) => str,
  [RuleLevel.Warn]: chalk.yellow,
  [RuleLevel.Error]: chalk.red,
};

const cli = new Command();
cli.name(pkg.name);
cli.version(pkg.version);

cli.option("-p, --path [string]", "path to source directory", cwd());
cli.option("-l, --log-level [number]", "from 0 to 2", parseInt, 0);

cli.parse(argv);

const options = cli.opts<ICliOptions>();

(async () => {
  chdir(options.path);
  const files = await globby(["**/*.wxml", "**/*.wxss"], { expandDirectories: true });

  const jobs = files
    .map((filename) => async () => {
      let wxss = "";
      let wxml = "";
      const fileContent = (await readFile(filename)).toString();
      if (filename.endsWith("wxss")) {
        wxss = fileContent;
      } else if (filename.endsWith("wxml")) {
        wxml = fileContent;
      }
      for (const result of parse({ wxml, wxss, rules })) {
        const { name, level, results } = result;
        if (options.logLevel > level) continue;
        const color = logColor[level];
        for (const result of results) {
          const { loc, advice, description } = result;
          let filePath = "";
          if (loc) {
            filePath = format("%s:%d:%d", chalk.blue(filename), loc.startLn, loc.startCol);
          } else {
            filePath = format("%s", chalk.blue(filename));
          }
          stdout.write(format("@%s %s\n", color(name), filePath));
          stdout.write(format("  %s", color(description)));
          advice && stdout.write(format("  💡 %s", chalk.gray(advice)));
          stdout.write("\n");
        }
      }
      stdout.write("\n");
    })
    .map((fn) => fn());

  Promise.allSettled(jobs);
})();
