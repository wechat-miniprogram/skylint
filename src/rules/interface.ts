import { type Node as WXMLNode } from "../walker/html";
import { type Node as WXSSNode } from "../walker/css";

export const enum RuleLevel {
  Verbose,
  Warn,
  Error,
}

export type SourceCodeLocation = Record<"startLn" | "endLn" | "startCol" | "endCol", number>;

export interface RuleResultItem {
  description?: string;
  advice?: string;
  loc?: SourceCodeLocation;
}

export const enum RuleType {
  Unknown,
  WXML,
  WXSS,
  Node,
}

interface HookType {
  [RuleType.WXML]: WXMLNode;
  [RuleType.WXSS]: WXSSNode;
  [RuleType.Node]: any;
  [RuleType.Unknown]: never;
}

interface Hooks<T extends RuleType> {
  before?: () => void;
  onVisit?: (node: HookType[T]) => boolean | void;
  after?: () => void;
}

interface RuleBasicInfo {
  name: string;
  type: RuleType;
  level: RuleLevel;
}

export interface Rule<T extends RuleType> extends Hooks<T>, RuleBasicInfo {
  get results(): RuleResultItem[];
}

// export const defineRule = <T extends RuleType>(name: string, type: T, hooks: Hooks<T>): Rule<T> => {
//   return {
//     name,
//     type,
//     ...hooks,
//   };
// };

export const defineRule = <T extends RuleType>(
  info: Pick<RuleBasicInfo, "name" | "type"> & Partial<Pick<RuleBasicInfo, "level">>,
  init: (ctx: { lifetimes(hooks: Hooks<T>): void; addResult(...results: RuleResultItem[]): void }) => void
): Rule<T> => {
  const { name, type, level = RuleLevel.Warn } = info;
  let hooks: Hooks<T> = {};
  let results: RuleResultItem[] = [];
  const lifetimes = (newHooks: Hooks<T>) => {
    hooks = { ...hooks, ...newHooks };
  };
  const addResult = (...newResults: RuleResultItem[]) => results.push(...newResults);
  init({
    lifetimes,
    addResult,
  });
  return {
    name,
    type,
    level,
    get results() {
      return results;
    },
    ...hooks,
  };
};
