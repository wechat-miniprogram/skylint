import { type Node as WXMLNode } from "../walker/html";
import { type Node as WXSSNode } from "../walker/css";
import { type Node as JSONNode } from "../walker/json";
import { Patch, PatchStatus } from "../patch";

export const enum RuleLevel {
  Verbose,
  Warn,
  Error,
}

export type SourceCodeLocation = Record<"startLn" | "endLn" | "startCol" | "endCol", number>;

export interface RuleResultItem {
  subname: string;
  description?: string;
  advice?: string;
  patchHint?: string;
  loc?: SourceCodeLocation;
}

export const enum RuleType {
  Unknown,
  WXML,
  WXSS,
  Node,
  JSON,
}

interface HookType {
  [RuleType.WXML]: WXMLNode;
  [RuleType.WXSS]: WXSSNode;
  [RuleType.JSON]: JSONNode;
  [RuleType.Node]: any;
  [RuleType.Unknown]: never;
}

interface Hooks<T extends RuleType> {
  before?: () => void;
  onVisit?: (node: HookType[T]) => boolean | void;
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

export interface RuleContext<T extends RuleType> {
  lifetimes(hooks: Hooks<T>): void;
  addResult(...results: RuleResultItem[]): void;
  addPatch(...patches: QuickPatch[]): void;
  addASTPatch(...patches: Function[]): void;
}

type QuickPatch = Pick<Patch, "loc" | "patchedStr">;

export type RuleBasicInfoWithOptionalLevel<T extends RuleType> = Pick<RuleBasicInfo<T>, "name" | "type"> &
  Partial<Pick<RuleBasicInfo<T>, "level">>;

export const defineRule =
  <T extends RuleType>(info: RuleBasicInfoWithOptionalLevel<T>, init: (ctx: RuleContext<T>) => void) =>
  () => {
    const { name, type, level = RuleLevel.Warn } = info;
    let hooks: Hooks<T> = {};
    let results: RuleResultItem[] = [];
    let patches: Patch[] = [];
    let astPatches: Function[] = [];
    const lifetimes = (newHooks: Hooks<T>) => {
      hooks = { ...hooks, ...newHooks };
    };
    const addResult = (...newResults: RuleResultItem[]) => results.push(...newResults);
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
    });
    return {
      name,
      type,
      level,
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
