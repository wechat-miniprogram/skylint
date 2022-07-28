import { HTMLWalker, type Node as WXMLNode } from "src/walker/html";
import { CSSWalker, type Node as WXSSNode } from "src/walker/css";
import { JSONWalker, type Node as JSONNode } from "src/walker/json";
import { Patch, PatchStatus } from "../patch";
import { BasicParseEnv } from "src/parser";
import { Document } from "domhandler";

export enum RuleLevel {
  Verbose = 0,
  Info = 1,
  Warn = 2,
  Error = 3,
}

interface BasicLocation {
  path: string | null;
}

export interface LocationLnColBased extends BasicLocation {
  startLn: number;
  endLn: number;
  startCol: number;
  endCol: number;
}

export interface LocationIndexBased extends BasicLocation {
  startIndex: number;
  endIndex: number;
}

export type SourceCodeLocation = LocationIndexBased | LocationLnColBased;

export interface RuleResultItem {
  name: string;
  description?: string;
  advice?: string;
  patchHint?: string;
  loc?: SourceCodeLocation;
  level: RuleLevel;
  fixable?: boolean;
  withCodeFrame?: boolean;
}

export const enum RuleType {
  Unknown,
  WXML,
  WXSS,
  Node,
  JSON,
}

interface HookType {
  [RuleType.WXML]: Parameters<HTMLWalker>[1];
  [RuleType.WXSS]: Parameters<CSSWalker>[1];
  [RuleType.JSON]: Parameters<JSONWalker>[1];
  [RuleType.Node]: any;
  [RuleType.Unknown]: never;
}

interface Hooks<T extends RuleType> {
  before?: () => void;
  onVisit?: HookType[T];
  after?: () => void;
}

interface RuleBasicInfo<T extends RuleType = RuleType> {
  name: string;
  type: T;
  level: RuleLevel;
}

export interface Rule<T extends RuleType = RuleType> extends Hooks<T>, RuleBasicInfo {
  get results(): RuleResultItem[];
  get patches(): Patch[];
  get astPatches(): Function[];
  clear(): void;
}

export interface RuleContext<T extends RuleType, K> {
  lifetimes(hooks: Hooks<T>): void;
  addResult(...results: RuleResultItem[]): void;
  addResultWithPatch(result: RuleResultItem, patch: QuickPatch): void;
  addPatch(...patches: Omit<Patch, "status">[]): void;
  addASTPatch(...patches: Function[]): void;
  getRelatedWXMLFilename(): string | undefined;
  getRelatedWXMLAst(): Document | null;
  env: K;
}

type QuickPatch = Pick<Patch, "loc" | "patchedStr">;

export type RuleBasicInfoWithOptionalLevel<T extends RuleType> = Pick<RuleBasicInfo<T>, "name" | "type">;

export const defineRule =
  <Env extends BasicParseEnv, T extends RuleType = RuleType>(
    info: RuleBasicInfoWithOptionalLevel<T>,
    init: (ctx: RuleContext<T, Env>) => void
  ) =>
  (env: Env) => {
    const { name, type } = info;
    let hooks: Hooks<T> = {};
    let results: RuleResultItem[] = [];
    let patches: Patch[] = [];
    let astPatches: Function[] = [];
    const lifetimes = (newHooks: Hooks<T>) => {
      hooks = { ...hooks, ...newHooks };
    };
    const addResult = (...newResults: RuleResultItem[]) => {
      results.push(...newResults);
    };
    const addResultWithPatch = (result: RuleResultItem, patch: Patch) => {
      const { name } = result;
      results.push(result);
      patches.push({
        ...patch,
        name,
        status: PatchStatus.Pending,
      });
    };
    const addASTPatch = (...newPatches: Function[]) => astPatches.push(...newPatches);
    const addPatch = (...newPatches: Omit<Patch, "status">[]) =>
      patches.push(
        ...newPatches.map<Patch>((patch) => ({
          ...patch,
          status: PatchStatus.Pending,
        }))
      );
    const getRelatedWXMLFilename = () => env.path.replace(/(?!\.)(wxml|json|wxss)$/, "wxml");

    const getRelatedWXMLAst = () => {
      if (!env.astMap) return null;
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
      env,
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
        results;
      },
      ...hooks,
    } as Rule<T>;
  };

export const createResultItem = (
  params: Omit<RuleResultItem, "level"> & Partial<Exclude<RuleResultItem, "level">>
): RuleResultItem => {
  return {
    level: RuleLevel.Warn,
    ...params,
  };
};
