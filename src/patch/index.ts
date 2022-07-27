import MagicString from "magic-string";

export interface PatchLocation {
  start: number;
  end: number;
  path: string;
}

export const enum PatchStatus {
  Pending,
  Applied,
  Failed,
}

export const enum PatchType {
  Replace,
  Append,
}

export interface Patch {
  //   type: PatchType;
  loc: PatchLocation;
  patchedStr: string;
  status: PatchStatus;
}

const sortPatchesByLoc = (patches: Patch[]) =>
  patches.sort((a, b) => {
    return a.loc.start - b.loc.start;
  });

const findOverlaps = (sortedPatches: Patch[]) => {
  const patches = sortedPatches;
  const endLocs = patches.map((r) => r.loc.end).sort((a, b) => a - b);
  let i = 0;
  let j = 0;
  let n = patches.length;
  let active = 0;
  const groups = [];
  let curGroup = [];
  while (true) {
    if (i < n && patches[i].loc.start <= endLocs[j]) {
      curGroup.push(patches[i++]);
      active++;
    } else if (j < n) {
      j++;
      if (--active === 0) {
        groups.push(curGroup);
        curGroup = [];
      }
    } else {
      break;
    }
  }
  return groups;
};

export const applyPatchesOnString = (rawString: string, patches: Patch[]) => {
  const str = new MagicString(rawString);
  const sortedPatches = sortPatchesByLoc(patches);
  //   const groups = findOverlaps(sortedPatches);
  //   const overlappedGroups: Patch[][] = [];
  const nonOverLappedPatches: Patch[] = sortedPatches;
  //   groups.forEach((group) => {
  //     if (group.length > 1) {
  //       overlappedGroups.push(group);
  //     } else {
  //       nonOverLappedPatches.push(...group);
  //     }
  //   });
  const len = nonOverLappedPatches.length;
  for (let i = 0; i < len; i++) {
    const { loc, patchedStr } = sortedPatches[i];
    const range = loc.end - loc.start;
    if (range === 0) {
      str.appendRight(loc.start, patchedStr);
    } else if (range > 0) {
      str.overwrite(loc.start, loc.end, patchedStr);
    }
    // // we don't maintain offset delta, it's magic-string's job
    // const delta = patchedStr.length - range;
    // for (let j = i + 1; j < len; j++) {
    //   const patch = nonOverLappedPatches[j];
    //   patch.loc.start += delta;
    //   patch.loc.end += delta;
    // }
  }
  return str;
};
