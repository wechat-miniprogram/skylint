import { HTMLWalker, type Node as WXMLNode } from "../walker/html";
import { CSSWalker, type Node as WXSSNode } from "../walker/css";
import { JSONWalker, type Node as JSONNode } from "../walker/json";
import { Patch, PatchStatus } from "../patch";

export const enum RuleLevel {
  Verbose = 0,
  Info = 1,
  Warn = 2,
  Error = 3,
}

interface LocationLnColBased {
  startLn: number;
  endLn: number;
  startCol: number;
  endCol: number;
}

interface LocationIndexBased {
  startIndex: number;
  endIndex: number;
}

export type SourceCodeLocation = LocationIndexBased | LocationLnColBased;

export interface RuleResultItem {
  subname: string;
  description?: string;
  advice?: string;
  patchHint?: string;
  loc?: SourceCodeLocation;
  level: RuleLevel;
  fixable?: boolean;
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
  addPatch(...patches: QuickPatch[]): void;
  addASTPatch(...patches: Function[]): void;
  env?: K;
}

type QuickPatch = Pick<Patch, "loc" | "patchedStr">;

export type RuleBasicInfoWithOptionalLevel<T extends RuleType> = Pick<RuleBasicInfo<T>, "name" | "type">;

export const defineRule =
  <Env, T extends RuleType = RuleType>(
    info: RuleBasicInfoWithOptionalLevel<T>,
    init: (ctx: RuleContext<T, Env>) => void
  ) =>
  (env?: Env) => {
    const { name, type } = info;
    let hooks: Hooks<T> = {};
    let results: RuleResultItem[] = [];
    let patches: Patch[] = [];
    let astPatches: Function[] = [];
    const lifetimes = (newHooks: Hooks<T>) => {
      hooks = { ...hooks, ...newHooks };
    };
    const addResult = (...newResults: Omit<RuleResultItem[], "level">) => {
      console.log(info.name, info.type, results)
      results.push(...newResults);
    };
    const addASTPatch = (...newPatches: Function[]) => astPatches.push(...newPatches);
    const addPatch = (...newPatches: QuickPatch[]) =>
      patches.push(
        ...newPatches.map<Patch>((patch) => ({
          ...patch,
          status: PatchStatus.Pending,
        }))
      );
    init({
      lifetimes,
      addASTPatch,
      addPatch,
      addResult,
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
