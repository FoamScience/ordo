var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getProtoOf = Object.getPrototypeOf;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __commonJS = (cb, mod) => function __require() {
  return mod || (0, cb[__getOwnPropNames(cb)[0]])((mod = { exports: {} }).exports, mod), mod.exports;
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
  // If the importer is in node compatibility mode or this is not an ESM
  // file that has been converted to a CommonJS file using a Babel-
  // compatible transform (i.e. "__esModule" has not been set), then set
  // "default" to the CommonJS "module.exports" for node compatibility.
  isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
  mod
));

// node_modules/diff-match-patch/index.js
var require_diff_match_patch = __commonJS({
  "node_modules/diff-match-patch/index.js"(exports, module) {
    var diff_match_patch = function() {
      this.Diff_Timeout = 1;
      this.Diff_EditCost = 4;
      this.Match_Threshold = 0.5;
      this.Match_Distance = 1e3;
      this.Patch_DeleteThreshold = 0.5;
      this.Patch_Margin = 4;
      this.Match_MaxBits = 32;
    };
    var DIFF_DELETE = -1;
    var DIFF_INSERT = 1;
    var DIFF_EQUAL = 0;
    diff_match_patch.Diff = function(op, text) {
      return [op, text];
    };
    diff_match_patch.prototype.diff_main = function(text1, text2, opt_checklines, opt_deadline) {
      if (typeof opt_deadline == "undefined") {
        if (this.Diff_Timeout <= 0) {
          opt_deadline = Number.MAX_VALUE;
        } else {
          opt_deadline = (/* @__PURE__ */ new Date()).getTime() + this.Diff_Timeout * 1e3;
        }
      }
      var deadline = opt_deadline;
      if (text1 == null || text2 == null) {
        throw new Error("Null input. (diff_main)");
      }
      if (text1 == text2) {
        if (text1) {
          return [new diff_match_patch.Diff(DIFF_EQUAL, text1)];
        }
        return [];
      }
      if (typeof opt_checklines == "undefined") {
        opt_checklines = true;
      }
      var checklines = opt_checklines;
      var commonlength = this.diff_commonPrefix(text1, text2);
      var commonprefix = text1.substring(0, commonlength);
      text1 = text1.substring(commonlength);
      text2 = text2.substring(commonlength);
      commonlength = this.diff_commonSuffix(text1, text2);
      var commonsuffix = text1.substring(text1.length - commonlength);
      text1 = text1.substring(0, text1.length - commonlength);
      text2 = text2.substring(0, text2.length - commonlength);
      var diffs = this.diff_compute_(text1, text2, checklines, deadline);
      if (commonprefix) {
        diffs.unshift(new diff_match_patch.Diff(DIFF_EQUAL, commonprefix));
      }
      if (commonsuffix) {
        diffs.push(new diff_match_patch.Diff(DIFF_EQUAL, commonsuffix));
      }
      this.diff_cleanupMerge(diffs);
      return diffs;
    };
    diff_match_patch.prototype.diff_compute_ = function(text1, text2, checklines, deadline) {
      var diffs;
      if (!text1) {
        return [new diff_match_patch.Diff(DIFF_INSERT, text2)];
      }
      if (!text2) {
        return [new diff_match_patch.Diff(DIFF_DELETE, text1)];
      }
      var longtext = text1.length > text2.length ? text1 : text2;
      var shorttext = text1.length > text2.length ? text2 : text1;
      var i = longtext.indexOf(shorttext);
      if (i != -1) {
        diffs = [
          new diff_match_patch.Diff(DIFF_INSERT, longtext.substring(0, i)),
          new diff_match_patch.Diff(DIFF_EQUAL, shorttext),
          new diff_match_patch.Diff(
            DIFF_INSERT,
            longtext.substring(i + shorttext.length)
          )
        ];
        if (text1.length > text2.length) {
          diffs[0][0] = diffs[2][0] = DIFF_DELETE;
        }
        return diffs;
      }
      if (shorttext.length == 1) {
        return [
          new diff_match_patch.Diff(DIFF_DELETE, text1),
          new diff_match_patch.Diff(DIFF_INSERT, text2)
        ];
      }
      var hm = this.diff_halfMatch_(text1, text2);
      if (hm) {
        var text1_a = hm[0];
        var text1_b = hm[1];
        var text2_a = hm[2];
        var text2_b = hm[3];
        var mid_common = hm[4];
        var diffs_a = this.diff_main(text1_a, text2_a, checklines, deadline);
        var diffs_b = this.diff_main(text1_b, text2_b, checklines, deadline);
        return diffs_a.concat(
          [new diff_match_patch.Diff(DIFF_EQUAL, mid_common)],
          diffs_b
        );
      }
      if (checklines && text1.length > 100 && text2.length > 100) {
        return this.diff_lineMode_(text1, text2, deadline);
      }
      return this.diff_bisect_(text1, text2, deadline);
    };
    diff_match_patch.prototype.diff_lineMode_ = function(text1, text2, deadline) {
      var a = this.diff_linesToChars_(text1, text2);
      text1 = a.chars1;
      text2 = a.chars2;
      var linearray = a.lineArray;
      var diffs = this.diff_main(text1, text2, false, deadline);
      this.diff_charsToLines_(diffs, linearray);
      this.diff_cleanupSemantic(diffs);
      diffs.push(new diff_match_patch.Diff(DIFF_EQUAL, ""));
      var pointer = 0;
      var count_delete = 0;
      var count_insert = 0;
      var text_delete = "";
      var text_insert = "";
      while (pointer < diffs.length) {
        switch (diffs[pointer][0]) {
          case DIFF_INSERT:
            count_insert++;
            text_insert += diffs[pointer][1];
            break;
          case DIFF_DELETE:
            count_delete++;
            text_delete += diffs[pointer][1];
            break;
          case DIFF_EQUAL:
            if (count_delete >= 1 && count_insert >= 1) {
              diffs.splice(
                pointer - count_delete - count_insert,
                count_delete + count_insert
              );
              pointer = pointer - count_delete - count_insert;
              var subDiff = this.diff_main(text_delete, text_insert, false, deadline);
              for (var j = subDiff.length - 1; j >= 0; j--) {
                diffs.splice(pointer, 0, subDiff[j]);
              }
              pointer = pointer + subDiff.length;
            }
            count_insert = 0;
            count_delete = 0;
            text_delete = "";
            text_insert = "";
            break;
        }
        pointer++;
      }
      diffs.pop();
      return diffs;
    };
    diff_match_patch.prototype.diff_bisect_ = function(text1, text2, deadline) {
      var text1_length = text1.length;
      var text2_length = text2.length;
      var max_d = Math.ceil((text1_length + text2_length) / 2);
      var v_offset = max_d;
      var v_length = 2 * max_d;
      var v1 = new Array(v_length);
      var v2 = new Array(v_length);
      for (var x = 0; x < v_length; x++) {
        v1[x] = -1;
        v2[x] = -1;
      }
      v1[v_offset + 1] = 0;
      v2[v_offset + 1] = 0;
      var delta = text1_length - text2_length;
      var front = delta % 2 != 0;
      var k1start = 0;
      var k1end = 0;
      var k2start = 0;
      var k2end = 0;
      for (var d = 0; d < max_d; d++) {
        if ((/* @__PURE__ */ new Date()).getTime() > deadline) {
          break;
        }
        for (var k1 = -d + k1start; k1 <= d - k1end; k1 += 2) {
          var k1_offset = v_offset + k1;
          var x1;
          if (k1 == -d || k1 != d && v1[k1_offset - 1] < v1[k1_offset + 1]) {
            x1 = v1[k1_offset + 1];
          } else {
            x1 = v1[k1_offset - 1] + 1;
          }
          var y1 = x1 - k1;
          while (x1 < text1_length && y1 < text2_length && text1.charAt(x1) == text2.charAt(y1)) {
            x1++;
            y1++;
          }
          v1[k1_offset] = x1;
          if (x1 > text1_length) {
            k1end += 2;
          } else if (y1 > text2_length) {
            k1start += 2;
          } else if (front) {
            var k2_offset = v_offset + delta - k1;
            if (k2_offset >= 0 && k2_offset < v_length && v2[k2_offset] != -1) {
              var x2 = text1_length - v2[k2_offset];
              if (x1 >= x2) {
                return this.diff_bisectSplit_(text1, text2, x1, y1, deadline);
              }
            }
          }
        }
        for (var k2 = -d + k2start; k2 <= d - k2end; k2 += 2) {
          var k2_offset = v_offset + k2;
          var x2;
          if (k2 == -d || k2 != d && v2[k2_offset - 1] < v2[k2_offset + 1]) {
            x2 = v2[k2_offset + 1];
          } else {
            x2 = v2[k2_offset - 1] + 1;
          }
          var y2 = x2 - k2;
          while (x2 < text1_length && y2 < text2_length && text1.charAt(text1_length - x2 - 1) == text2.charAt(text2_length - y2 - 1)) {
            x2++;
            y2++;
          }
          v2[k2_offset] = x2;
          if (x2 > text1_length) {
            k2end += 2;
          } else if (y2 > text2_length) {
            k2start += 2;
          } else if (!front) {
            var k1_offset = v_offset + delta - k2;
            if (k1_offset >= 0 && k1_offset < v_length && v1[k1_offset] != -1) {
              var x1 = v1[k1_offset];
              var y1 = v_offset + x1 - k1_offset;
              x2 = text1_length - x2;
              if (x1 >= x2) {
                return this.diff_bisectSplit_(text1, text2, x1, y1, deadline);
              }
            }
          }
        }
      }
      return [
        new diff_match_patch.Diff(DIFF_DELETE, text1),
        new diff_match_patch.Diff(DIFF_INSERT, text2)
      ];
    };
    diff_match_patch.prototype.diff_bisectSplit_ = function(text1, text2, x, y, deadline) {
      var text1a = text1.substring(0, x);
      var text2a = text2.substring(0, y);
      var text1b = text1.substring(x);
      var text2b = text2.substring(y);
      var diffs = this.diff_main(text1a, text2a, false, deadline);
      var diffsb = this.diff_main(text1b, text2b, false, deadline);
      return diffs.concat(diffsb);
    };
    diff_match_patch.prototype.diff_linesToChars_ = function(text1, text2) {
      var lineArray = [];
      var lineHash = {};
      lineArray[0] = "";
      function diff_linesToCharsMunge_(text) {
        var chars = "";
        var lineStart = 0;
        var lineEnd = -1;
        var lineArrayLength = lineArray.length;
        while (lineEnd < text.length - 1) {
          lineEnd = text.indexOf("\n", lineStart);
          if (lineEnd == -1) {
            lineEnd = text.length - 1;
          }
          var line = text.substring(lineStart, lineEnd + 1);
          if (lineHash.hasOwnProperty ? lineHash.hasOwnProperty(line) : lineHash[line] !== void 0) {
            chars += String.fromCharCode(lineHash[line]);
          } else {
            if (lineArrayLength == maxLines) {
              line = text.substring(lineStart);
              lineEnd = text.length;
            }
            chars += String.fromCharCode(lineArrayLength);
            lineHash[line] = lineArrayLength;
            lineArray[lineArrayLength++] = line;
          }
          lineStart = lineEnd + 1;
        }
        return chars;
      }
      var maxLines = 4e4;
      var chars1 = diff_linesToCharsMunge_(text1);
      maxLines = 65535;
      var chars2 = diff_linesToCharsMunge_(text2);
      return { chars1, chars2, lineArray };
    };
    diff_match_patch.prototype.diff_charsToLines_ = function(diffs, lineArray) {
      for (var i = 0; i < diffs.length; i++) {
        var chars = diffs[i][1];
        var text = [];
        for (var j = 0; j < chars.length; j++) {
          text[j] = lineArray[chars.charCodeAt(j)];
        }
        diffs[i][1] = text.join("");
      }
    };
    diff_match_patch.prototype.diff_commonPrefix = function(text1, text2) {
      if (!text1 || !text2 || text1.charAt(0) != text2.charAt(0)) {
        return 0;
      }
      var pointermin = 0;
      var pointermax = Math.min(text1.length, text2.length);
      var pointermid = pointermax;
      var pointerstart = 0;
      while (pointermin < pointermid) {
        if (text1.substring(pointerstart, pointermid) == text2.substring(pointerstart, pointermid)) {
          pointermin = pointermid;
          pointerstart = pointermin;
        } else {
          pointermax = pointermid;
        }
        pointermid = Math.floor((pointermax - pointermin) / 2 + pointermin);
      }
      return pointermid;
    };
    diff_match_patch.prototype.diff_commonSuffix = function(text1, text2) {
      if (!text1 || !text2 || text1.charAt(text1.length - 1) != text2.charAt(text2.length - 1)) {
        return 0;
      }
      var pointermin = 0;
      var pointermax = Math.min(text1.length, text2.length);
      var pointermid = pointermax;
      var pointerend = 0;
      while (pointermin < pointermid) {
        if (text1.substring(text1.length - pointermid, text1.length - pointerend) == text2.substring(text2.length - pointermid, text2.length - pointerend)) {
          pointermin = pointermid;
          pointerend = pointermin;
        } else {
          pointermax = pointermid;
        }
        pointermid = Math.floor((pointermax - pointermin) / 2 + pointermin);
      }
      return pointermid;
    };
    diff_match_patch.prototype.diff_commonOverlap_ = function(text1, text2) {
      var text1_length = text1.length;
      var text2_length = text2.length;
      if (text1_length == 0 || text2_length == 0) {
        return 0;
      }
      if (text1_length > text2_length) {
        text1 = text1.substring(text1_length - text2_length);
      } else if (text1_length < text2_length) {
        text2 = text2.substring(0, text1_length);
      }
      var text_length = Math.min(text1_length, text2_length);
      if (text1 == text2) {
        return text_length;
      }
      var best = 0;
      var length = 1;
      while (true) {
        var pattern = text1.substring(text_length - length);
        var found = text2.indexOf(pattern);
        if (found == -1) {
          return best;
        }
        length += found;
        if (found == 0 || text1.substring(text_length - length) == text2.substring(0, length)) {
          best = length;
          length++;
        }
      }
    };
    diff_match_patch.prototype.diff_halfMatch_ = function(text1, text2) {
      if (this.Diff_Timeout <= 0) {
        return null;
      }
      var longtext = text1.length > text2.length ? text1 : text2;
      var shorttext = text1.length > text2.length ? text2 : text1;
      if (longtext.length < 4 || shorttext.length * 2 < longtext.length) {
        return null;
      }
      var dmp2 = this;
      function diff_halfMatchI_(longtext2, shorttext2, i) {
        var seed = longtext2.substring(i, i + Math.floor(longtext2.length / 4));
        var j = -1;
        var best_common = "";
        var best_longtext_a, best_longtext_b, best_shorttext_a, best_shorttext_b;
        while ((j = shorttext2.indexOf(seed, j + 1)) != -1) {
          var prefixLength = dmp2.diff_commonPrefix(
            longtext2.substring(i),
            shorttext2.substring(j)
          );
          var suffixLength = dmp2.diff_commonSuffix(
            longtext2.substring(0, i),
            shorttext2.substring(0, j)
          );
          if (best_common.length < suffixLength + prefixLength) {
            best_common = shorttext2.substring(j - suffixLength, j) + shorttext2.substring(j, j + prefixLength);
            best_longtext_a = longtext2.substring(0, i - suffixLength);
            best_longtext_b = longtext2.substring(i + prefixLength);
            best_shorttext_a = shorttext2.substring(0, j - suffixLength);
            best_shorttext_b = shorttext2.substring(j + prefixLength);
          }
        }
        if (best_common.length * 2 >= longtext2.length) {
          return [
            best_longtext_a,
            best_longtext_b,
            best_shorttext_a,
            best_shorttext_b,
            best_common
          ];
        } else {
          return null;
        }
      }
      var hm1 = diff_halfMatchI_(
        longtext,
        shorttext,
        Math.ceil(longtext.length / 4)
      );
      var hm2 = diff_halfMatchI_(
        longtext,
        shorttext,
        Math.ceil(longtext.length / 2)
      );
      var hm;
      if (!hm1 && !hm2) {
        return null;
      } else if (!hm2) {
        hm = hm1;
      } else if (!hm1) {
        hm = hm2;
      } else {
        hm = hm1[4].length > hm2[4].length ? hm1 : hm2;
      }
      var text1_a, text1_b, text2_a, text2_b;
      if (text1.length > text2.length) {
        text1_a = hm[0];
        text1_b = hm[1];
        text2_a = hm[2];
        text2_b = hm[3];
      } else {
        text2_a = hm[0];
        text2_b = hm[1];
        text1_a = hm[2];
        text1_b = hm[3];
      }
      var mid_common = hm[4];
      return [text1_a, text1_b, text2_a, text2_b, mid_common];
    };
    diff_match_patch.prototype.diff_cleanupSemantic = function(diffs) {
      var changes = false;
      var equalities = [];
      var equalitiesLength = 0;
      var lastEquality = null;
      var pointer = 0;
      var length_insertions1 = 0;
      var length_deletions1 = 0;
      var length_insertions2 = 0;
      var length_deletions2 = 0;
      while (pointer < diffs.length) {
        if (diffs[pointer][0] == DIFF_EQUAL) {
          equalities[equalitiesLength++] = pointer;
          length_insertions1 = length_insertions2;
          length_deletions1 = length_deletions2;
          length_insertions2 = 0;
          length_deletions2 = 0;
          lastEquality = diffs[pointer][1];
        } else {
          if (diffs[pointer][0] == DIFF_INSERT) {
            length_insertions2 += diffs[pointer][1].length;
          } else {
            length_deletions2 += diffs[pointer][1].length;
          }
          if (lastEquality && lastEquality.length <= Math.max(length_insertions1, length_deletions1) && lastEquality.length <= Math.max(
            length_insertions2,
            length_deletions2
          )) {
            diffs.splice(
              equalities[equalitiesLength - 1],
              0,
              new diff_match_patch.Diff(DIFF_DELETE, lastEquality)
            );
            diffs[equalities[equalitiesLength - 1] + 1][0] = DIFF_INSERT;
            equalitiesLength--;
            equalitiesLength--;
            pointer = equalitiesLength > 0 ? equalities[equalitiesLength - 1] : -1;
            length_insertions1 = 0;
            length_deletions1 = 0;
            length_insertions2 = 0;
            length_deletions2 = 0;
            lastEquality = null;
            changes = true;
          }
        }
        pointer++;
      }
      if (changes) {
        this.diff_cleanupMerge(diffs);
      }
      this.diff_cleanupSemanticLossless(diffs);
      pointer = 1;
      while (pointer < diffs.length) {
        if (diffs[pointer - 1][0] == DIFF_DELETE && diffs[pointer][0] == DIFF_INSERT) {
          var deletion = diffs[pointer - 1][1];
          var insertion = diffs[pointer][1];
          var overlap_length1 = this.diff_commonOverlap_(deletion, insertion);
          var overlap_length2 = this.diff_commonOverlap_(insertion, deletion);
          if (overlap_length1 >= overlap_length2) {
            if (overlap_length1 >= deletion.length / 2 || overlap_length1 >= insertion.length / 2) {
              diffs.splice(pointer, 0, new diff_match_patch.Diff(
                DIFF_EQUAL,
                insertion.substring(0, overlap_length1)
              ));
              diffs[pointer - 1][1] = deletion.substring(0, deletion.length - overlap_length1);
              diffs[pointer + 1][1] = insertion.substring(overlap_length1);
              pointer++;
            }
          } else {
            if (overlap_length2 >= deletion.length / 2 || overlap_length2 >= insertion.length / 2) {
              diffs.splice(pointer, 0, new diff_match_patch.Diff(
                DIFF_EQUAL,
                deletion.substring(0, overlap_length2)
              ));
              diffs[pointer - 1][0] = DIFF_INSERT;
              diffs[pointer - 1][1] = insertion.substring(0, insertion.length - overlap_length2);
              diffs[pointer + 1][0] = DIFF_DELETE;
              diffs[pointer + 1][1] = deletion.substring(overlap_length2);
              pointer++;
            }
          }
          pointer++;
        }
        pointer++;
      }
    };
    diff_match_patch.prototype.diff_cleanupSemanticLossless = function(diffs) {
      function diff_cleanupSemanticScore_(one, two) {
        if (!one || !two) {
          return 6;
        }
        var char1 = one.charAt(one.length - 1);
        var char2 = two.charAt(0);
        var nonAlphaNumeric1 = char1.match(diff_match_patch.nonAlphaNumericRegex_);
        var nonAlphaNumeric2 = char2.match(diff_match_patch.nonAlphaNumericRegex_);
        var whitespace1 = nonAlphaNumeric1 && char1.match(diff_match_patch.whitespaceRegex_);
        var whitespace2 = nonAlphaNumeric2 && char2.match(diff_match_patch.whitespaceRegex_);
        var lineBreak1 = whitespace1 && char1.match(diff_match_patch.linebreakRegex_);
        var lineBreak2 = whitespace2 && char2.match(diff_match_patch.linebreakRegex_);
        var blankLine1 = lineBreak1 && one.match(diff_match_patch.blanklineEndRegex_);
        var blankLine2 = lineBreak2 && two.match(diff_match_patch.blanklineStartRegex_);
        if (blankLine1 || blankLine2) {
          return 5;
        } else if (lineBreak1 || lineBreak2) {
          return 4;
        } else if (nonAlphaNumeric1 && !whitespace1 && whitespace2) {
          return 3;
        } else if (whitespace1 || whitespace2) {
          return 2;
        } else if (nonAlphaNumeric1 || nonAlphaNumeric2) {
          return 1;
        }
        return 0;
      }
      var pointer = 1;
      while (pointer < diffs.length - 1) {
        if (diffs[pointer - 1][0] == DIFF_EQUAL && diffs[pointer + 1][0] == DIFF_EQUAL) {
          var equality1 = diffs[pointer - 1][1];
          var edit = diffs[pointer][1];
          var equality2 = diffs[pointer + 1][1];
          var commonOffset = this.diff_commonSuffix(equality1, edit);
          if (commonOffset) {
            var commonString = edit.substring(edit.length - commonOffset);
            equality1 = equality1.substring(0, equality1.length - commonOffset);
            edit = commonString + edit.substring(0, edit.length - commonOffset);
            equality2 = commonString + equality2;
          }
          var bestEquality1 = equality1;
          var bestEdit = edit;
          var bestEquality2 = equality2;
          var bestScore = diff_cleanupSemanticScore_(equality1, edit) + diff_cleanupSemanticScore_(edit, equality2);
          while (edit.charAt(0) === equality2.charAt(0)) {
            equality1 += edit.charAt(0);
            edit = edit.substring(1) + equality2.charAt(0);
            equality2 = equality2.substring(1);
            var score = diff_cleanupSemanticScore_(equality1, edit) + diff_cleanupSemanticScore_(edit, equality2);
            if (score >= bestScore) {
              bestScore = score;
              bestEquality1 = equality1;
              bestEdit = edit;
              bestEquality2 = equality2;
            }
          }
          if (diffs[pointer - 1][1] != bestEquality1) {
            if (bestEquality1) {
              diffs[pointer - 1][1] = bestEquality1;
            } else {
              diffs.splice(pointer - 1, 1);
              pointer--;
            }
            diffs[pointer][1] = bestEdit;
            if (bestEquality2) {
              diffs[pointer + 1][1] = bestEquality2;
            } else {
              diffs.splice(pointer + 1, 1);
              pointer--;
            }
          }
        }
        pointer++;
      }
    };
    diff_match_patch.nonAlphaNumericRegex_ = /[^a-zA-Z0-9]/;
    diff_match_patch.whitespaceRegex_ = /\s/;
    diff_match_patch.linebreakRegex_ = /[\r\n]/;
    diff_match_patch.blanklineEndRegex_ = /\n\r?\n$/;
    diff_match_patch.blanklineStartRegex_ = /^\r?\n\r?\n/;
    diff_match_patch.prototype.diff_cleanupEfficiency = function(diffs) {
      var changes = false;
      var equalities = [];
      var equalitiesLength = 0;
      var lastEquality = null;
      var pointer = 0;
      var pre_ins = false;
      var pre_del = false;
      var post_ins = false;
      var post_del = false;
      while (pointer < diffs.length) {
        if (diffs[pointer][0] == DIFF_EQUAL) {
          if (diffs[pointer][1].length < this.Diff_EditCost && (post_ins || post_del)) {
            equalities[equalitiesLength++] = pointer;
            pre_ins = post_ins;
            pre_del = post_del;
            lastEquality = diffs[pointer][1];
          } else {
            equalitiesLength = 0;
            lastEquality = null;
          }
          post_ins = post_del = false;
        } else {
          if (diffs[pointer][0] == DIFF_DELETE) {
            post_del = true;
          } else {
            post_ins = true;
          }
          if (lastEquality && (pre_ins && pre_del && post_ins && post_del || lastEquality.length < this.Diff_EditCost / 2 && pre_ins + pre_del + post_ins + post_del == 3)) {
            diffs.splice(
              equalities[equalitiesLength - 1],
              0,
              new diff_match_patch.Diff(DIFF_DELETE, lastEquality)
            );
            diffs[equalities[equalitiesLength - 1] + 1][0] = DIFF_INSERT;
            equalitiesLength--;
            lastEquality = null;
            if (pre_ins && pre_del) {
              post_ins = post_del = true;
              equalitiesLength = 0;
            } else {
              equalitiesLength--;
              pointer = equalitiesLength > 0 ? equalities[equalitiesLength - 1] : -1;
              post_ins = post_del = false;
            }
            changes = true;
          }
        }
        pointer++;
      }
      if (changes) {
        this.diff_cleanupMerge(diffs);
      }
    };
    diff_match_patch.prototype.diff_cleanupMerge = function(diffs) {
      diffs.push(new diff_match_patch.Diff(DIFF_EQUAL, ""));
      var pointer = 0;
      var count_delete = 0;
      var count_insert = 0;
      var text_delete = "";
      var text_insert = "";
      var commonlength;
      while (pointer < diffs.length) {
        switch (diffs[pointer][0]) {
          case DIFF_INSERT:
            count_insert++;
            text_insert += diffs[pointer][1];
            pointer++;
            break;
          case DIFF_DELETE:
            count_delete++;
            text_delete += diffs[pointer][1];
            pointer++;
            break;
          case DIFF_EQUAL:
            if (count_delete + count_insert > 1) {
              if (count_delete !== 0 && count_insert !== 0) {
                commonlength = this.diff_commonPrefix(text_insert, text_delete);
                if (commonlength !== 0) {
                  if (pointer - count_delete - count_insert > 0 && diffs[pointer - count_delete - count_insert - 1][0] == DIFF_EQUAL) {
                    diffs[pointer - count_delete - count_insert - 1][1] += text_insert.substring(0, commonlength);
                  } else {
                    diffs.splice(0, 0, new diff_match_patch.Diff(
                      DIFF_EQUAL,
                      text_insert.substring(0, commonlength)
                    ));
                    pointer++;
                  }
                  text_insert = text_insert.substring(commonlength);
                  text_delete = text_delete.substring(commonlength);
                }
                commonlength = this.diff_commonSuffix(text_insert, text_delete);
                if (commonlength !== 0) {
                  diffs[pointer][1] = text_insert.substring(text_insert.length - commonlength) + diffs[pointer][1];
                  text_insert = text_insert.substring(0, text_insert.length - commonlength);
                  text_delete = text_delete.substring(0, text_delete.length - commonlength);
                }
              }
              pointer -= count_delete + count_insert;
              diffs.splice(pointer, count_delete + count_insert);
              if (text_delete.length) {
                diffs.splice(
                  pointer,
                  0,
                  new diff_match_patch.Diff(DIFF_DELETE, text_delete)
                );
                pointer++;
              }
              if (text_insert.length) {
                diffs.splice(
                  pointer,
                  0,
                  new diff_match_patch.Diff(DIFF_INSERT, text_insert)
                );
                pointer++;
              }
              pointer++;
            } else if (pointer !== 0 && diffs[pointer - 1][0] == DIFF_EQUAL) {
              diffs[pointer - 1][1] += diffs[pointer][1];
              diffs.splice(pointer, 1);
            } else {
              pointer++;
            }
            count_insert = 0;
            count_delete = 0;
            text_delete = "";
            text_insert = "";
            break;
        }
      }
      if (diffs[diffs.length - 1][1] === "") {
        diffs.pop();
      }
      var changes = false;
      pointer = 1;
      while (pointer < diffs.length - 1) {
        if (diffs[pointer - 1][0] == DIFF_EQUAL && diffs[pointer + 1][0] == DIFF_EQUAL) {
          if (diffs[pointer][1].substring(diffs[pointer][1].length - diffs[pointer - 1][1].length) == diffs[pointer - 1][1]) {
            diffs[pointer][1] = diffs[pointer - 1][1] + diffs[pointer][1].substring(0, diffs[pointer][1].length - diffs[pointer - 1][1].length);
            diffs[pointer + 1][1] = diffs[pointer - 1][1] + diffs[pointer + 1][1];
            diffs.splice(pointer - 1, 1);
            changes = true;
          } else if (diffs[pointer][1].substring(0, diffs[pointer + 1][1].length) == diffs[pointer + 1][1]) {
            diffs[pointer - 1][1] += diffs[pointer + 1][1];
            diffs[pointer][1] = diffs[pointer][1].substring(diffs[pointer + 1][1].length) + diffs[pointer + 1][1];
            diffs.splice(pointer + 1, 1);
            changes = true;
          }
        }
        pointer++;
      }
      if (changes) {
        this.diff_cleanupMerge(diffs);
      }
    };
    diff_match_patch.prototype.diff_xIndex = function(diffs, loc) {
      var chars1 = 0;
      var chars2 = 0;
      var last_chars1 = 0;
      var last_chars2 = 0;
      var x;
      for (x = 0; x < diffs.length; x++) {
        if (diffs[x][0] !== DIFF_INSERT) {
          chars1 += diffs[x][1].length;
        }
        if (diffs[x][0] !== DIFF_DELETE) {
          chars2 += diffs[x][1].length;
        }
        if (chars1 > loc) {
          break;
        }
        last_chars1 = chars1;
        last_chars2 = chars2;
      }
      if (diffs.length != x && diffs[x][0] === DIFF_DELETE) {
        return last_chars2;
      }
      return last_chars2 + (loc - last_chars1);
    };
    diff_match_patch.prototype.diff_prettyHtml = function(diffs) {
      var html = [];
      var pattern_amp = /&/g;
      var pattern_lt = /</g;
      var pattern_gt = />/g;
      var pattern_para = /\n/g;
      for (var x = 0; x < diffs.length; x++) {
        var op = diffs[x][0];
        var data = diffs[x][1];
        var text = data.replace(pattern_amp, "&amp;").replace(pattern_lt, "&lt;").replace(pattern_gt, "&gt;").replace(pattern_para, "&para;<br>");
        switch (op) {
          case DIFF_INSERT:
            html[x] = '<ins style="background:#e6ffe6;">' + text + "</ins>";
            break;
          case DIFF_DELETE:
            html[x] = '<del style="background:#ffe6e6;">' + text + "</del>";
            break;
          case DIFF_EQUAL:
            html[x] = "<span>" + text + "</span>";
            break;
        }
      }
      return html.join("");
    };
    diff_match_patch.prototype.diff_text1 = function(diffs) {
      var text = [];
      for (var x = 0; x < diffs.length; x++) {
        if (diffs[x][0] !== DIFF_INSERT) {
          text[x] = diffs[x][1];
        }
      }
      return text.join("");
    };
    diff_match_patch.prototype.diff_text2 = function(diffs) {
      var text = [];
      for (var x = 0; x < diffs.length; x++) {
        if (diffs[x][0] !== DIFF_DELETE) {
          text[x] = diffs[x][1];
        }
      }
      return text.join("");
    };
    diff_match_patch.prototype.diff_levenshtein = function(diffs) {
      var levenshtein = 0;
      var insertions = 0;
      var deletions = 0;
      for (var x = 0; x < diffs.length; x++) {
        var op = diffs[x][0];
        var data = diffs[x][1];
        switch (op) {
          case DIFF_INSERT:
            insertions += data.length;
            break;
          case DIFF_DELETE:
            deletions += data.length;
            break;
          case DIFF_EQUAL:
            levenshtein += Math.max(insertions, deletions);
            insertions = 0;
            deletions = 0;
            break;
        }
      }
      levenshtein += Math.max(insertions, deletions);
      return levenshtein;
    };
    diff_match_patch.prototype.diff_toDelta = function(diffs) {
      var text = [];
      for (var x = 0; x < diffs.length; x++) {
        switch (diffs[x][0]) {
          case DIFF_INSERT:
            text[x] = "+" + encodeURI(diffs[x][1]);
            break;
          case DIFF_DELETE:
            text[x] = "-" + diffs[x][1].length;
            break;
          case DIFF_EQUAL:
            text[x] = "=" + diffs[x][1].length;
            break;
        }
      }
      return text.join("	").replace(/%20/g, " ");
    };
    diff_match_patch.prototype.diff_fromDelta = function(text1, delta) {
      var diffs = [];
      var diffsLength = 0;
      var pointer = 0;
      var tokens = delta.split(/\t/g);
      for (var x = 0; x < tokens.length; x++) {
        var param = tokens[x].substring(1);
        switch (tokens[x].charAt(0)) {
          case "+":
            try {
              diffs[diffsLength++] = new diff_match_patch.Diff(DIFF_INSERT, decodeURI(param));
            } catch (ex) {
              throw new Error("Illegal escape in diff_fromDelta: " + param);
            }
            break;
          case "-":
          // Fall through.
          case "=":
            var n = parseInt(param, 10);
            if (isNaN(n) || n < 0) {
              throw new Error("Invalid number in diff_fromDelta: " + param);
            }
            var text = text1.substring(pointer, pointer += n);
            if (tokens[x].charAt(0) == "=") {
              diffs[diffsLength++] = new diff_match_patch.Diff(DIFF_EQUAL, text);
            } else {
              diffs[diffsLength++] = new diff_match_patch.Diff(DIFF_DELETE, text);
            }
            break;
          default:
            if (tokens[x]) {
              throw new Error("Invalid diff operation in diff_fromDelta: " + tokens[x]);
            }
        }
      }
      if (pointer != text1.length) {
        throw new Error("Delta length (" + pointer + ") does not equal source text length (" + text1.length + ").");
      }
      return diffs;
    };
    diff_match_patch.prototype.match_main = function(text, pattern, loc) {
      if (text == null || pattern == null || loc == null) {
        throw new Error("Null input. (match_main)");
      }
      loc = Math.max(0, Math.min(loc, text.length));
      if (text == pattern) {
        return 0;
      } else if (!text.length) {
        return -1;
      } else if (text.substring(loc, loc + pattern.length) == pattern) {
        return loc;
      } else {
        return this.match_bitap_(text, pattern, loc);
      }
    };
    diff_match_patch.prototype.match_bitap_ = function(text, pattern, loc) {
      if (pattern.length > this.Match_MaxBits) {
        throw new Error("Pattern too long for this browser.");
      }
      var s = this.match_alphabet_(pattern);
      var dmp2 = this;
      function match_bitapScore_(e, x) {
        var accuracy = e / pattern.length;
        var proximity = Math.abs(loc - x);
        if (!dmp2.Match_Distance) {
          return proximity ? 1 : accuracy;
        }
        return accuracy + proximity / dmp2.Match_Distance;
      }
      var score_threshold = this.Match_Threshold;
      var best_loc = text.indexOf(pattern, loc);
      if (best_loc != -1) {
        score_threshold = Math.min(match_bitapScore_(0, best_loc), score_threshold);
        best_loc = text.lastIndexOf(pattern, loc + pattern.length);
        if (best_loc != -1) {
          score_threshold = Math.min(match_bitapScore_(0, best_loc), score_threshold);
        }
      }
      var matchmask = 1 << pattern.length - 1;
      best_loc = -1;
      var bin_min, bin_mid;
      var bin_max = pattern.length + text.length;
      var last_rd;
      for (var d = 0; d < pattern.length; d++) {
        bin_min = 0;
        bin_mid = bin_max;
        while (bin_min < bin_mid) {
          if (match_bitapScore_(d, loc + bin_mid) <= score_threshold) {
            bin_min = bin_mid;
          } else {
            bin_max = bin_mid;
          }
          bin_mid = Math.floor((bin_max - bin_min) / 2 + bin_min);
        }
        bin_max = bin_mid;
        var start = Math.max(1, loc - bin_mid + 1);
        var finish = Math.min(loc + bin_mid, text.length) + pattern.length;
        var rd = Array(finish + 2);
        rd[finish + 1] = (1 << d) - 1;
        for (var j = finish; j >= start; j--) {
          var charMatch = s[text.charAt(j - 1)];
          if (d === 0) {
            rd[j] = (rd[j + 1] << 1 | 1) & charMatch;
          } else {
            rd[j] = (rd[j + 1] << 1 | 1) & charMatch | ((last_rd[j + 1] | last_rd[j]) << 1 | 1) | last_rd[j + 1];
          }
          if (rd[j] & matchmask) {
            var score = match_bitapScore_(d, j - 1);
            if (score <= score_threshold) {
              score_threshold = score;
              best_loc = j - 1;
              if (best_loc > loc) {
                start = Math.max(1, 2 * loc - best_loc);
              } else {
                break;
              }
            }
          }
        }
        if (match_bitapScore_(d + 1, loc) > score_threshold) {
          break;
        }
        last_rd = rd;
      }
      return best_loc;
    };
    diff_match_patch.prototype.match_alphabet_ = function(pattern) {
      var s = {};
      for (var i = 0; i < pattern.length; i++) {
        s[pattern.charAt(i)] = 0;
      }
      for (var i = 0; i < pattern.length; i++) {
        s[pattern.charAt(i)] |= 1 << pattern.length - i - 1;
      }
      return s;
    };
    diff_match_patch.prototype.patch_addContext_ = function(patch, text) {
      if (text.length == 0) {
        return;
      }
      if (patch.start2 === null) {
        throw Error("patch not initialized");
      }
      var pattern = text.substring(patch.start2, patch.start2 + patch.length1);
      var padding = 0;
      while (text.indexOf(pattern) != text.lastIndexOf(pattern) && pattern.length < this.Match_MaxBits - this.Patch_Margin - this.Patch_Margin) {
        padding += this.Patch_Margin;
        pattern = text.substring(
          patch.start2 - padding,
          patch.start2 + patch.length1 + padding
        );
      }
      padding += this.Patch_Margin;
      var prefix = text.substring(patch.start2 - padding, patch.start2);
      if (prefix) {
        patch.diffs.unshift(new diff_match_patch.Diff(DIFF_EQUAL, prefix));
      }
      var suffix = text.substring(
        patch.start2 + patch.length1,
        patch.start2 + patch.length1 + padding
      );
      if (suffix) {
        patch.diffs.push(new diff_match_patch.Diff(DIFF_EQUAL, suffix));
      }
      patch.start1 -= prefix.length;
      patch.start2 -= prefix.length;
      patch.length1 += prefix.length + suffix.length;
      patch.length2 += prefix.length + suffix.length;
    };
    diff_match_patch.prototype.patch_make = function(a, opt_b, opt_c) {
      var text1, diffs;
      if (typeof a == "string" && typeof opt_b == "string" && typeof opt_c == "undefined") {
        text1 = /** @type {string} */
        a;
        diffs = this.diff_main(
          text1,
          /** @type {string} */
          opt_b,
          true
        );
        if (diffs.length > 2) {
          this.diff_cleanupSemantic(diffs);
          this.diff_cleanupEfficiency(diffs);
        }
      } else if (a && typeof a == "object" && typeof opt_b == "undefined" && typeof opt_c == "undefined") {
        diffs = /** @type {!Array.<!diff_match_patch.Diff>} */
        a;
        text1 = this.diff_text1(diffs);
      } else if (typeof a == "string" && opt_b && typeof opt_b == "object" && typeof opt_c == "undefined") {
        text1 = /** @type {string} */
        a;
        diffs = /** @type {!Array.<!diff_match_patch.Diff>} */
        opt_b;
      } else if (typeof a == "string" && typeof opt_b == "string" && opt_c && typeof opt_c == "object") {
        text1 = /** @type {string} */
        a;
        diffs = /** @type {!Array.<!diff_match_patch.Diff>} */
        opt_c;
      } else {
        throw new Error("Unknown call format to patch_make.");
      }
      if (diffs.length === 0) {
        return [];
      }
      var patches = [];
      var patch = new diff_match_patch.patch_obj();
      var patchDiffLength = 0;
      var char_count1 = 0;
      var char_count2 = 0;
      var prepatch_text = text1;
      var postpatch_text = text1;
      for (var x = 0; x < diffs.length; x++) {
        var diff_type = diffs[x][0];
        var diff_text = diffs[x][1];
        if (!patchDiffLength && diff_type !== DIFF_EQUAL) {
          patch.start1 = char_count1;
          patch.start2 = char_count2;
        }
        switch (diff_type) {
          case DIFF_INSERT:
            patch.diffs[patchDiffLength++] = diffs[x];
            patch.length2 += diff_text.length;
            postpatch_text = postpatch_text.substring(0, char_count2) + diff_text + postpatch_text.substring(char_count2);
            break;
          case DIFF_DELETE:
            patch.length1 += diff_text.length;
            patch.diffs[patchDiffLength++] = diffs[x];
            postpatch_text = postpatch_text.substring(0, char_count2) + postpatch_text.substring(char_count2 + diff_text.length);
            break;
          case DIFF_EQUAL:
            if (diff_text.length <= 2 * this.Patch_Margin && patchDiffLength && diffs.length != x + 1) {
              patch.diffs[patchDiffLength++] = diffs[x];
              patch.length1 += diff_text.length;
              patch.length2 += diff_text.length;
            } else if (diff_text.length >= 2 * this.Patch_Margin) {
              if (patchDiffLength) {
                this.patch_addContext_(patch, prepatch_text);
                patches.push(patch);
                patch = new diff_match_patch.patch_obj();
                patchDiffLength = 0;
                prepatch_text = postpatch_text;
                char_count1 = char_count2;
              }
            }
            break;
        }
        if (diff_type !== DIFF_INSERT) {
          char_count1 += diff_text.length;
        }
        if (diff_type !== DIFF_DELETE) {
          char_count2 += diff_text.length;
        }
      }
      if (patchDiffLength) {
        this.patch_addContext_(patch, prepatch_text);
        patches.push(patch);
      }
      return patches;
    };
    diff_match_patch.prototype.patch_deepCopy = function(patches) {
      var patchesCopy = [];
      for (var x = 0; x < patches.length; x++) {
        var patch = patches[x];
        var patchCopy = new diff_match_patch.patch_obj();
        patchCopy.diffs = [];
        for (var y = 0; y < patch.diffs.length; y++) {
          patchCopy.diffs[y] = new diff_match_patch.Diff(patch.diffs[y][0], patch.diffs[y][1]);
        }
        patchCopy.start1 = patch.start1;
        patchCopy.start2 = patch.start2;
        patchCopy.length1 = patch.length1;
        patchCopy.length2 = patch.length2;
        patchesCopy[x] = patchCopy;
      }
      return patchesCopy;
    };
    diff_match_patch.prototype.patch_apply = function(patches, text) {
      if (patches.length == 0) {
        return [text, []];
      }
      patches = this.patch_deepCopy(patches);
      var nullPadding = this.patch_addPadding(patches);
      text = nullPadding + text + nullPadding;
      this.patch_splitMax(patches);
      var delta = 0;
      var results = [];
      for (var x = 0; x < patches.length; x++) {
        var expected_loc = patches[x].start2 + delta;
        var text1 = this.diff_text1(patches[x].diffs);
        var start_loc;
        var end_loc = -1;
        if (text1.length > this.Match_MaxBits) {
          start_loc = this.match_main(
            text,
            text1.substring(0, this.Match_MaxBits),
            expected_loc
          );
          if (start_loc != -1) {
            end_loc = this.match_main(
              text,
              text1.substring(text1.length - this.Match_MaxBits),
              expected_loc + text1.length - this.Match_MaxBits
            );
            if (end_loc == -1 || start_loc >= end_loc) {
              start_loc = -1;
            }
          }
        } else {
          start_loc = this.match_main(text, text1, expected_loc);
        }
        if (start_loc == -1) {
          results[x] = false;
          delta -= patches[x].length2 - patches[x].length1;
        } else {
          results[x] = true;
          delta = start_loc - expected_loc;
          var text2;
          if (end_loc == -1) {
            text2 = text.substring(start_loc, start_loc + text1.length);
          } else {
            text2 = text.substring(start_loc, end_loc + this.Match_MaxBits);
          }
          if (text1 == text2) {
            text = text.substring(0, start_loc) + this.diff_text2(patches[x].diffs) + text.substring(start_loc + text1.length);
          } else {
            var diffs = this.diff_main(text1, text2, false);
            if (text1.length > this.Match_MaxBits && this.diff_levenshtein(diffs) / text1.length > this.Patch_DeleteThreshold) {
              results[x] = false;
            } else {
              this.diff_cleanupSemanticLossless(diffs);
              var index1 = 0;
              var index2;
              for (var y = 0; y < patches[x].diffs.length; y++) {
                var mod = patches[x].diffs[y];
                if (mod[0] !== DIFF_EQUAL) {
                  index2 = this.diff_xIndex(diffs, index1);
                }
                if (mod[0] === DIFF_INSERT) {
                  text = text.substring(0, start_loc + index2) + mod[1] + text.substring(start_loc + index2);
                } else if (mod[0] === DIFF_DELETE) {
                  text = text.substring(0, start_loc + index2) + text.substring(start_loc + this.diff_xIndex(
                    diffs,
                    index1 + mod[1].length
                  ));
                }
                if (mod[0] !== DIFF_DELETE) {
                  index1 += mod[1].length;
                }
              }
            }
          }
        }
      }
      text = text.substring(nullPadding.length, text.length - nullPadding.length);
      return [text, results];
    };
    diff_match_patch.prototype.patch_addPadding = function(patches) {
      var paddingLength = this.Patch_Margin;
      var nullPadding = "";
      for (var x = 1; x <= paddingLength; x++) {
        nullPadding += String.fromCharCode(x);
      }
      for (var x = 0; x < patches.length; x++) {
        patches[x].start1 += paddingLength;
        patches[x].start2 += paddingLength;
      }
      var patch = patches[0];
      var diffs = patch.diffs;
      if (diffs.length == 0 || diffs[0][0] != DIFF_EQUAL) {
        diffs.unshift(new diff_match_patch.Diff(DIFF_EQUAL, nullPadding));
        patch.start1 -= paddingLength;
        patch.start2 -= paddingLength;
        patch.length1 += paddingLength;
        patch.length2 += paddingLength;
      } else if (paddingLength > diffs[0][1].length) {
        var extraLength = paddingLength - diffs[0][1].length;
        diffs[0][1] = nullPadding.substring(diffs[0][1].length) + diffs[0][1];
        patch.start1 -= extraLength;
        patch.start2 -= extraLength;
        patch.length1 += extraLength;
        patch.length2 += extraLength;
      }
      patch = patches[patches.length - 1];
      diffs = patch.diffs;
      if (diffs.length == 0 || diffs[diffs.length - 1][0] != DIFF_EQUAL) {
        diffs.push(new diff_match_patch.Diff(DIFF_EQUAL, nullPadding));
        patch.length1 += paddingLength;
        patch.length2 += paddingLength;
      } else if (paddingLength > diffs[diffs.length - 1][1].length) {
        var extraLength = paddingLength - diffs[diffs.length - 1][1].length;
        diffs[diffs.length - 1][1] += nullPadding.substring(0, extraLength);
        patch.length1 += extraLength;
        patch.length2 += extraLength;
      }
      return nullPadding;
    };
    diff_match_patch.prototype.patch_splitMax = function(patches) {
      var patch_size = this.Match_MaxBits;
      for (var x = 0; x < patches.length; x++) {
        if (patches[x].length1 <= patch_size) {
          continue;
        }
        var bigpatch = patches[x];
        patches.splice(x--, 1);
        var start1 = bigpatch.start1;
        var start2 = bigpatch.start2;
        var precontext = "";
        while (bigpatch.diffs.length !== 0) {
          var patch = new diff_match_patch.patch_obj();
          var empty = true;
          patch.start1 = start1 - precontext.length;
          patch.start2 = start2 - precontext.length;
          if (precontext !== "") {
            patch.length1 = patch.length2 = precontext.length;
            patch.diffs.push(new diff_match_patch.Diff(DIFF_EQUAL, precontext));
          }
          while (bigpatch.diffs.length !== 0 && patch.length1 < patch_size - this.Patch_Margin) {
            var diff_type = bigpatch.diffs[0][0];
            var diff_text = bigpatch.diffs[0][1];
            if (diff_type === DIFF_INSERT) {
              patch.length2 += diff_text.length;
              start2 += diff_text.length;
              patch.diffs.push(bigpatch.diffs.shift());
              empty = false;
            } else if (diff_type === DIFF_DELETE && patch.diffs.length == 1 && patch.diffs[0][0] == DIFF_EQUAL && diff_text.length > 2 * patch_size) {
              patch.length1 += diff_text.length;
              start1 += diff_text.length;
              empty = false;
              patch.diffs.push(new diff_match_patch.Diff(diff_type, diff_text));
              bigpatch.diffs.shift();
            } else {
              diff_text = diff_text.substring(
                0,
                patch_size - patch.length1 - this.Patch_Margin
              );
              patch.length1 += diff_text.length;
              start1 += diff_text.length;
              if (diff_type === DIFF_EQUAL) {
                patch.length2 += diff_text.length;
                start2 += diff_text.length;
              } else {
                empty = false;
              }
              patch.diffs.push(new diff_match_patch.Diff(diff_type, diff_text));
              if (diff_text == bigpatch.diffs[0][1]) {
                bigpatch.diffs.shift();
              } else {
                bigpatch.diffs[0][1] = bigpatch.diffs[0][1].substring(diff_text.length);
              }
            }
          }
          precontext = this.diff_text2(patch.diffs);
          precontext = precontext.substring(precontext.length - this.Patch_Margin);
          var postcontext = this.diff_text1(bigpatch.diffs).substring(0, this.Patch_Margin);
          if (postcontext !== "") {
            patch.length1 += postcontext.length;
            patch.length2 += postcontext.length;
            if (patch.diffs.length !== 0 && patch.diffs[patch.diffs.length - 1][0] === DIFF_EQUAL) {
              patch.diffs[patch.diffs.length - 1][1] += postcontext;
            } else {
              patch.diffs.push(new diff_match_patch.Diff(DIFF_EQUAL, postcontext));
            }
          }
          if (!empty) {
            patches.splice(++x, 0, patch);
          }
        }
      }
    };
    diff_match_patch.prototype.patch_toText = function(patches) {
      var text = [];
      for (var x = 0; x < patches.length; x++) {
        text[x] = patches[x];
      }
      return text.join("");
    };
    diff_match_patch.prototype.patch_fromText = function(textline) {
      var patches = [];
      if (!textline) {
        return patches;
      }
      var text = textline.split("\n");
      var textPointer = 0;
      var patchHeader = /^@@ -(\d+),?(\d*) \+(\d+),?(\d*) @@$/;
      while (textPointer < text.length) {
        var m = text[textPointer].match(patchHeader);
        if (!m) {
          throw new Error("Invalid patch string: " + text[textPointer]);
        }
        var patch = new diff_match_patch.patch_obj();
        patches.push(patch);
        patch.start1 = parseInt(m[1], 10);
        if (m[2] === "") {
          patch.start1--;
          patch.length1 = 1;
        } else if (m[2] == "0") {
          patch.length1 = 0;
        } else {
          patch.start1--;
          patch.length1 = parseInt(m[2], 10);
        }
        patch.start2 = parseInt(m[3], 10);
        if (m[4] === "") {
          patch.start2--;
          patch.length2 = 1;
        } else if (m[4] == "0") {
          patch.length2 = 0;
        } else {
          patch.start2--;
          patch.length2 = parseInt(m[4], 10);
        }
        textPointer++;
        while (textPointer < text.length) {
          var sign = text[textPointer].charAt(0);
          try {
            var line = decodeURI(text[textPointer].substring(1));
          } catch (ex) {
            throw new Error("Illegal escape in patch_fromText: " + line);
          }
          if (sign == "-") {
            patch.diffs.push(new diff_match_patch.Diff(DIFF_DELETE, line));
          } else if (sign == "+") {
            patch.diffs.push(new diff_match_patch.Diff(DIFF_INSERT, line));
          } else if (sign == " ") {
            patch.diffs.push(new diff_match_patch.Diff(DIFF_EQUAL, line));
          } else if (sign == "@") {
            break;
          } else if (sign === "") {
          } else {
            throw new Error('Invalid patch mode "' + sign + '" in: ' + line);
          }
          textPointer++;
        }
      }
      return patches;
    };
    diff_match_patch.patch_obj = function() {
      this.diffs = [];
      this.start1 = null;
      this.start2 = null;
      this.length1 = 0;
      this.length2 = 0;
    };
    diff_match_patch.patch_obj.prototype.toString = function() {
      var coords1, coords2;
      if (this.length1 === 0) {
        coords1 = this.start1 + ",0";
      } else if (this.length1 == 1) {
        coords1 = this.start1 + 1;
      } else {
        coords1 = this.start1 + 1 + "," + this.length1;
      }
      if (this.length2 === 0) {
        coords2 = this.start2 + ",0";
      } else if (this.length2 == 1) {
        coords2 = this.start2 + 1;
      } else {
        coords2 = this.start2 + 1 + "," + this.length2;
      }
      var text = ["@@ -" + coords1 + " +" + coords2 + " @@\n"];
      var op;
      for (var x = 0; x < this.diffs.length; x++) {
        switch (this.diffs[x][0]) {
          case DIFF_INSERT:
            op = "+";
            break;
          case DIFF_DELETE:
            op = "-";
            break;
          case DIFF_EQUAL:
            op = " ";
            break;
        }
        text[x + 1] = op + encodeURI(this.diffs[x][1]) + "\n";
      }
      return text.join("").replace(/%20/g, " ");
    };
    module.exports = diff_match_patch;
    module.exports["diff_match_patch"] = diff_match_patch;
    module.exports["DIFF_DELETE"] = DIFF_DELETE;
    module.exports["DIFF_INSERT"] = DIFF_INSERT;
    module.exports["DIFF_EQUAL"] = DIFF_EQUAL;
  }
});

// src/api.ts
var Api = class {
  token;
  rev;
  constructor(token2, rev2) {
    this.token = token2;
    this.rev = rev2;
  }
  url(path, params = {}) {
    const q = new URLSearchParams({ rev: this.rev });
    for (const [k, v] of Object.entries(params)) q.set(k, String(v));
    return `${path}?${q}`;
  }
  async get(path, params = {}) {
    const res = await fetch(this.url(path, params), { headers: { "X-Ordo-Token": this.token } });
    return answer(res);
  }
  review() {
    return this.get("/api/review");
  }
  file(path) {
    return this.get("/api/file", { path });
  }
  item(i) {
    return this.get("/api/item", { i });
  }
  themes() {
    return this.get("/api/themes");
  }
  async act(body) {
    const res = await fetch(this.url("/api/act"), {
      method: "POST",
      headers: { "X-Ordo-Token": this.token, "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
    return answer(res);
  }
  events(on) {
    const es = new EventSource(this.url("/api/events", { t: this.token }));
    for (const name of ["progress", "changed"]) {
      es.addEventListener(name, (e) => on(name, JSON.parse(e.data)));
    }
    return es;
  }
};
async function answer(res) {
  const text = await res.text();
  let body = null;
  try {
    body = JSON.parse(text);
  } catch {
  }
  if (!res.ok) throw new Error(body?.error ?? `${res.status} ${res.statusText}`);
  if (body === null) throw new Error("the server sent something that is not JSON");
  return body;
}

// src/dom.ts
function h(tag, attrs = null, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs ?? {})) {
    if (v === null || v === void 0 || v === false) continue;
    if (k.startsWith("on") && typeof v === "function") {
      el.addEventListener(k.slice(2), v);
    } else if (k === "class") {
      el.className = String(v);
    } else if (k === "dataset") {
      Object.assign(el.dataset, v);
    } else {
      el.setAttribute(k, v === true ? "" : String(v));
    }
  }
  append(el, children);
  return el;
}
function append(el, children) {
  for (const c of children) {
    if (c === null || c === void 0 || c === false) continue;
    if (Array.isArray(c)) append(el, c);
    else el.appendChild(typeof c === "object" ? c : document.createTextNode(String(c)));
  }
}
function $(sel) {
  const el = document.querySelector(sel);
  if (!el) throw new Error(`missing ${sel}`);
  return el;
}
function replace(parent, ...children) {
  parent.replaceChildren();
  append(parent, children);
}

// src/keys.ts
var NAMED = {
  " ": "Space",
  Escape: "Esc",
  Enter: "Enter",
  Backspace: "Backspace",
  Tab: "Tab",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown"
};
function keyLabel(e) {
  if (["Shift", "Control", "Alt", "Meta", "CapsLock", "Dead"].includes(e.key)) return null;
  if (e.metaKey) return null;
  const named = NAMED[e.key] ?? (/^F\d{1,2}$/.test(e.key) ? e.key : null);
  const char = !named && [...e.key].length === 1 ? e.key : null;
  if (!named && !char) return null;
  let prefix = "";
  if (e.ctrlKey) prefix += "C-";
  if (e.shiftKey && named && named !== "Space") prefix += "S-";
  if (e.altKey) prefix += "A-";
  return prefix + (named ?? char);
}
var Keymap = class {
  binds;
  constructor(binds) {
    this.binds = binds;
  }
  /** The terminal's rule: a key that opens a chord waits for the next one. */
  resolve(pending, key) {
    const find = (pre) => this.binds.find((b2) => b2.prefix === pre && b2.key === key);
    if (pending !== null) {
      const b2 = find(pending);
      return b2 ? { act: b2.action } : { miss: true };
    }
    if (this.binds.some((b2) => b2.prefix === key)) return { pending: true };
    const b = find(null);
    return b ? { act: b.action } : { miss: true };
  }
  /** What `key` does on its own, even when it also opens a chord. */
  bare(key) {
    return this.binds.find((b) => b.prefix === null && b.key === key)?.action ?? null;
  }
  /** Every chord bound to `action`, as `?` lists them. */
  keysFor(action) {
    return this.binds.filter((b) => b.action === action).map((b) => b.prefix ? `${b.prefix}${b.prefix.length > 1 || b.key.length > 1 ? " " : ""}${b.key}` : b.key);
  }
};

// src/layout.ts
var import_diff_match_patch = __toESM(require_diff_match_patch(), 1);
function centerScroll(rowTop, rowHeight2, viewHeight, contentHeight) {
  const want = rowTop + rowHeight2 / 2 - viewHeight / 2;
  const max = Math.max(0, contentHeight - viewHeight);
  return Math.round(Math.min(max, Math.max(0, want)));
}
function tableLimit(total, maximised) {
  return maximised ? total : Math.min(total, 3);
}
function codeRows(f) {
  const hunks = [...f.hunks].sort((a, b) => a.new[0] - b.new[0] || a.old[0] - b.old[0]);
  const rows = [];
  const startingAt = /* @__PURE__ */ new Map();
  for (const h2 of hunks) startingAt.set(h2.new[0], [...startingAt.get(h2.new[0]) ?? [], h2]);
  const added = /* @__PURE__ */ new Map();
  for (const h2 of hunks) {
    for (let n = h2.new[0]; n <= h2.new[1]; n++) {
      added.set(n, { hunk: h2.i, marks: h2.added[n - h2.new[0]] ?? null });
    }
  }
  const { folded, edited } = editsInPlace(f, hunks);
  const removedBefore = (n) => {
    for (const h2 of startingAt.get(n) ?? []) {
      for (let o = h2.old[0]; o <= h2.old[1]; o++) {
        if (folded.has(o)) continue;
        rows.push({
          kind: "del",
          o,
          text: f.old[o - 1] ?? "",
          spans: f.old_spans?.[o - 1] ?? null,
          hunk: h2.i,
          marks: h2.removed[o - h2.old[0]] ?? null
        });
      }
    }
  };
  for (let n = 1; n <= f.new.length; n++) {
    removedBefore(n);
    const a = added.get(n);
    const e = edited.get(n);
    rows.push({
      kind: a ? e ? "edit" : "add" : "ctx",
      n,
      text: f.new[n - 1],
      spans: f.new_spans?.[n - 1] ?? null,
      hunk: a?.hunk ?? null,
      marks: e ? e.marks : a?.marks ?? null,
      ...e && e.gone.length ? { gone: e.gone } : {}
    });
  }
  removedBefore(f.new.length + 1);
  return rows;
}
function editsInPlace(f, hunks) {
  const folded = /* @__PURE__ */ new Set();
  const edited = /* @__PURE__ */ new Map();
  for (const h2 of hunks) {
    let next = h2.new[0];
    for (let o = h2.old[0]; o <= h2.old[1]; o++) {
      const rm = h2.removed[o - h2.old[0]];
      const oldText = f.old[o - 1] ?? "";
      for (let n = next; n <= h2.new[1]; n++) {
        if (edited.has(n)) continue;
        const add = h2.added[n - h2.new[0]];
        const newText = f.new[n - 1] ?? "";
        const e = rm && add ? fromSpans(oldText, newText, rm, add) : !rm && !add ? byChars(oldText, newText) : null;
        if (e) {
          folded.add(o);
          edited.set(n, e);
          next = n + 1;
        }
        break;
      }
    }
  }
  return { folded, edited };
}
function fromSpans(oldText, newText, rm, add) {
  const o = [...oldText];
  const n = [...newText];
  const inside = (spans, k) => spans.some(([a, b]) => k >= a && k < b);
  const gone = [];
  let i = 0;
  let j = 0;
  while (i < o.length || j < n.length) {
    if (i < o.length && inside(rm, i)) {
      const from = i;
      while (i < o.length && inside(rm, i)) i++;
      gone.push([j, o.slice(from, i).join("")]);
    } else if (j < n.length && inside(add, j)) {
      j++;
    } else if (i < o.length && j < n.length && o[i] === n[j]) {
      i++;
      j++;
    } else {
      return null;
    }
  }
  return { marks: add, gone };
}
var dmp = new import_diff_match_patch.default();
var KEPT = 0.6;
function byChars(oldText, newText) {
  if (!oldText.trim() || !newText.trim()) return null;
  const diffs = dmp.diff_main(oldText, newText);
  dmp.diff_cleanupSemantic(diffs);
  const marks = [];
  const gone = [];
  let col = 0;
  let kept = 0;
  for (const [op, text] of diffs) {
    const len = [...text].length;
    if (op === import_diff_match_patch.default.DIFF_EQUAL) {
      kept += len;
      col += len;
    } else if (op === import_diff_match_patch.default.DIFF_INSERT) {
      marks.push([col, col + len]);
      col += len;
    } else {
      gone.push([col, text]);
    }
  }
  return kept >= KEPT * Math.max([...oldText].length, [...newText].length) ? { marks, gone } : null;
}
function firstRowOf(rows, hunk) {
  const k = rows.findIndex((r) => r.hunk === hunk);
  return k < 0 ? 0 : k;
}
function runs(text, spans, marks, gone = []) {
  const chars = [...text];
  const role = new Array(chars.length).fill(null);
  if (spans) {
    let p = 0;
    for (const [t, r] of spans) {
      for (let k = 0; k < [...t].length && p < chars.length; k++) role[p++] = r;
    }
  }
  const changed = new Array(chars.length).fill(false);
  for (const [a, b] of marks ?? []) for (let k = a; k < b && k < chars.length; k++) changed[k] = true;
  const out = [];
  const lost = (k) => {
    for (const [at, t] of gone) if (at === k) out.push({ text: t, role: null, changed: false, gone: true });
  };
  for (let k = 0; k < chars.length; k++) {
    lost(k);
    const last = out[out.length - 1];
    if (last && !last.gone && last.role === role[k] && last.changed === changed[k]) last.text += chars[k];
    else out.push({ text: chars[k], role: role[k], changed: changed[k] });
  }
  lost(chars.length);
  return out;
}
function wordClass(c) {
  if (/\s/u.test(c)) return 0;
  return /[\p{L}\p{N}_]/u.test(c) ? 1 : 2;
}
function flatten(lines) {
  const chars = [];
  const starts = [];
  lines.forEach((l, k) => {
    starts.push(chars.length);
    chars.push(...l);
    if (k + 1 < lines.length) chars.push("\n");
  });
  return { chars, starts };
}
function caretAt(lines, starts, i) {
  let row = 0;
  while (row + 1 < starts.length && starts[row + 1] <= i) row++;
  return { row, col: Math.min(i - starts[row], [...lines[row]].length) };
}
function wordMove(lines, at, kind) {
  const { chars, starts } = flatten(lines);
  if (!chars.length) return { row: 0, col: 0 };
  let i = Math.min((starts[at.row] ?? 0) + at.col, chars.length - 1);
  const cls = (k) => wordClass(chars[k]);
  if (kind === "next") {
    const c2 = cls(i);
    if (c2 !== 0) while (i + 1 < chars.length && cls(i + 1) === c2) i++;
    while (i + 1 < chars.length && cls(i + 1) === 0) i++;
    return caretAt(lines, starts, Math.min(i + 1, chars.length - 1));
  }
  if (kind === "prev") {
    if (i === 0) return { row: 0, col: 0 };
    i--;
    while (i > 0 && cls(i) === 0) i--;
    const c2 = cls(i);
    while (i > 0 && cls(i - 1) === c2) i--;
    return caretAt(lines, starts, i);
  }
  if (i + 1 >= chars.length) return caretAt(lines, starts, i);
  i++;
  while (i < chars.length && cls(i) === 0) i++;
  if (i >= chars.length) return caretAt(lines, starts, chars.length - 1);
  const c = cls(i);
  while (i + 1 < chars.length && cls(i + 1) === c) i++;
  return caretAt(lines, starts, i);
}

// src/palette.ts
function parseCommand(line) {
  const t = line.trim().replace(/^:/, "").trimStart();
  const sp = t.search(/\s/);
  return sp < 0 ? { name: t, arg: "" } : { name: t.slice(0, sp), arg: t.slice(sp + 1).trim() };
}
var CLIENT_COMMANDS = [
  { name: "theme", args: "<name>", help: "switch the colour theme (auto follows the system)" },
  { name: "size", args: "<small|normal|large|larger|factor>", help: "make the text and rows bigger or smaller, for this browser" }
];
function complete(input, commands, args) {
  const t = input.replace(/^:/, "");
  const sp = t.indexOf(" ");
  if (sp < 0) {
    return commands.map((c) => c.name).filter((n) => n.startsWith(t));
  }
  const name = t.slice(0, sp);
  const typed = t.slice(sp + 1);
  return (args[name] ?? []).filter((a) => a.startsWith(typed)).map((a) => `${name} ${a}`);
}

// src/theme.ts
var themes = {};
function cssVar(role) {
  if (role.startsWith("syntax-")) return `--syn-${role.slice(7)}`;
  const own = { border: "--line", "border-focus": "--focus" };
  if (own[role]) return own[role];
  return ["fg", "dim", "accent", "reviewed"].includes(role) || role.endsWith("-bg") ? `--${role}` : null;
}
function baseOf(theme) {
  const fg = theme["fg"];
  if (!fg || !/^#[0-9a-f]{6}$/i.test(fg)) return null;
  const n = parseInt(fg.slice(1), 16);
  const lum = (0.299 * (n >> 16) + 0.587 * (n >> 8 & 255) + 0.114 * (n & 255)) / 255;
  return lum > 0.5 ? "dark" : "light";
}
async function loadThemes(api2) {
  try {
    const t = await api2.themes();
    themes = t.themes;
  } catch {
    themes = {};
  }
}
function themeNames() {
  return Object.keys(themes).filter((n) => n !== "dark" && n !== "light");
}
function applyTheme(name) {
  const root = document.documentElement;
  for (const p of [...root.style]) if (p.startsWith("--") && p !== "--ui") root.style.removeProperty(p);
  root.removeAttribute("data-base");
  const t = themes[name];
  if (!t || name === "auto") return;
  const base = baseOf(t);
  if (base) root.dataset.base = base;
  for (const [role, value] of Object.entries(t)) {
    const v = cssVar(role);
    if (v && value) root.style.setProperty(v, value);
  }
}

// src/main.ts
var PANES = ["list", "code", "why"];
var LEVELS = ["verdict", "warn", "note"];
var SINCE = {
  new: ["\u2726", "new since you last looked"],
  changed: ["\u25D0", "changed since you last looked"],
  moved: ["\u21C5", "unchanged, but it reads in a different place now \u2014 its dependencies moved"]
};
var CHORD_TIMEOUT = 500;
var OP = { ctx: " ", add: "+", del: "-", edit: "~" };
var rev = new URLSearchParams(location.search).get("rev") ?? "HEAD";
var token = new URLSearchParams(location.hash.slice(1)).get("t") ?? sessionStorage.getItem("ordo.token") ?? "";
sessionStorage.setItem("ordo.token", token);
if (location.hash) history.replaceState(null, "", location.pathname + location.search);
var api = new Api(token, rev);
var st = {
  review: null,
  status: "starting\u2026",
  version: -1,
  keymap: new Keymap([]),
  sel: -1,
  focus: "list",
  zoom: false,
  file: null,
  rows: [],
  files: /* @__PURE__ */ new Map(),
  facts: null,
  cursor: 0,
  from: null,
  jumps: [],
  pending: null,
  pendingTimer: 0,
  search: null,
  sinceOnly: false,
  empty: false,
  col: null,
  whyLinks: [],
  whyCur: -1,
  notice: ""
};
var refreshSeq = 0;
var selectSeq = 0;
var filesGen = 0;
async function refresh() {
  const seq = ++refreshSeq;
  let slot;
  try {
    slot = await api.review();
  } catch (e) {
    if (seq !== refreshSeq) return;
    st.status = String(e.message);
    return renderAll();
  }
  if (seq !== refreshSeq) return;
  if (slot.state !== "ready") {
    st.status = slot.status;
    st.empty = slot.state === "empty";
    st.review = null;
    return renderAll();
  }
  st.empty = false;
  if (slot.version < st.version && slot.generation === st.review?.generation) return;
  const prev = st.review;
  st.review = slot;
  st.version = slot.version;
  st.keymap = new Keymap(slot.keys);
  const moved = !!prev && prev.generation !== slot.generation;
  const remap = (i) => {
    if (!prev || !moved) return i;
    const id = identity(prev.items[i]);
    return id ? slot.items.findIndex((it) => identity(it) === id) : -1;
  };
  let sel = st.sel >= 0 ? remap(st.sel) : -1;
  if (sel < 0) sel = prev ? clampToView(Math.min(st.sel, slot.items.length - 1)) : slot.sel;
  if (moved) {
    st.jumps = st.jumps.map((j) => ({ ...j, sel: remap(j.sel) })).filter((j) => j.sel >= 0);
    if (canvas) {
      const c = canvas;
      c.anchor = remap(c.anchor);
      c.cards = c.cards.map((k) => ({ ...k, i: remap(k.i) })).filter((k) => k.i >= 0);
      if (c.anchor < 0 || !c.cards.length) {
        canvas = null;
        closeSheet();
      } else {
        c.sel = Math.min(c.sel, c.cards.length - 1);
      }
    }
  }
  if (slot.delta.new + slot.delta.changed + slot.delta.moved === 0) st.sinceOnly = false;
  if (prev?.version !== slot.version) {
    st.files.clear();
    filesGen++;
    st.facts = null;
  }
  const same = sel === st.sel && !!st.file;
  if (same) st.sel = -1;
  await select(sel, { keepCursor: same });
  if (canvas) renderCanvas();
}
function identity(it) {
  return it ? `${it.path}\0${it.new[0]}\0${it.rationale}` : "";
}
function clampToView(i) {
  const v = st.review.view;
  return v.includes(i) ? i : v[0] ?? 0;
}
async function fileOf(path) {
  const hit = st.files.get(path);
  if (hit) return hit;
  const gen = filesGen;
  const f = await api.file(path);
  if (gen === filesGen) st.files.set(path, f);
  return f;
}
var factsFor = -1;
async function loadFacts(i) {
  factsFor = i;
  const version = st.version;
  try {
    const f = await api.item(i);
    if (factsFor === i && version === st.version) {
      st.facts = f;
      renderWhy();
      renderBand();
    }
  } catch {
  }
}
async function select(i, opts = {}) {
  const r = st.review;
  if (!r || i < 0 || i >= r.items.length) return renderAll();
  const seq = ++selectSeq;
  const it = r.items[i];
  let f;
  try {
    f = await fileOf(it.path);
  } catch (e) {
    if (seq === selectSeq) toast(e.message, true);
    return;
  }
  if (seq !== selectSeq || st.review !== r) return;
  const changed = i !== st.sel;
  if (changed && st.file?.path !== it.path) st.search = null;
  st.sel = i;
  if (opts.focus) st.focus = opts.focus;
  st.file = f.missing ? null : f;
  st.rows = st.file ? codeRows(st.file) : [];
  if (!opts.keepCursor) {
    st.col = null;
    st.cursor = firstRowOf(st.rows, i);
    st.from = null;
  }
  st.cursor = Math.min(st.cursor, Math.max(0, st.rows.length - 1));
  if (changed) {
    st.facts = null;
    st.whyCur = -1;
  }
  renderAll();
  scrollCodeTo(firstRowOf(st.rows, i), !opts.keepCursor || changed);
  loadFacts(i);
}
async function act(body) {
  try {
    const out = await api.act({
      sel: st.sel,
      generation: st.review?.generation,
      line: lineOfCursor(),
      from: fromLine(),
      ...body
    });
    if (out.error) toast(out.error, true);
    if (out.notice) toast(out.notice);
    if (out.popup) showSheet(out.popup.title, out.popup.lines.join("\n"));
    if (out.clipboard !== void 0) {
      await navigator.clipboard.writeText(out.clipboard).then(
        () => toast("copied the review prompt to the clipboard"),
        () => showSheet("review prompt", out.clipboard)
      );
    }
    if (out.navigate) {
      const q = new URLSearchParams({ rev: out.navigate });
      location.href = `/?${q}#t=${token}`;
    }
    if (out.version > st.version) await refresh();
    if (out.sel !== void 0 && out.sel !== st.sel) await select(out.sel);
    renderConfig();
    return out;
  } catch (e) {
    toast(e.message, true);
    refresh();
    return null;
  }
}
function lineOfCursor() {
  const row = st.rows[st.cursor];
  if (!row) return void 0;
  if (row.kind !== "del") return row.n - 1;
  const next = st.rows.slice(st.cursor).find((r) => r.kind !== "del");
  return next ? next.n - 1 : void 0;
}
function fromLine() {
  if (st.from === null) return void 0;
  const row = st.rows[st.from];
  return row && row.kind !== "del" ? row.n - 1 : void 0;
}
function shownRows() {
  const r = st.review;
  if (!st.sinceOnly) return r.rows;
  const keep = (i) => r.items[i].delta !== null;
  const out = [];
  for (const row of r.rows) {
    if (row.kind === "item" && !keep(row.i)) continue;
    if (row.kind === "folded" && !row.items.some(keep)) continue;
    if (row.kind === "gap") continue;
    if (row.kind === "file" && out.at(-1)?.kind === "file") out.pop();
    out.push(row);
  }
  if (out.at(-1)?.kind === "file") out.pop();
  return out;
}
function stops() {
  return shownRows().flatMap(
    (row) => row.kind === "item" ? [row.i] : row.kind === "folded" ? [row.items.includes(st.sel) ? st.sel : row.items[0]] : []
  );
}
function step(by) {
  const s = stops();
  if (!s.length) return;
  const at = s.indexOf(st.sel);
  const to = at < 0 ? 0 : Math.max(0, Math.min(s.length - 1, at + by));
  select(s[to]);
}
function rowClass(items, selected) {
  const r = st.review;
  const its = items.map((i) => r.items[i]);
  const reviewed = its.every((it) => it.reviewed);
  const level = LEVELS.find((l) => its.some((it) => it.level === l));
  const quiet = reviewed || its.every((it) => it.noise);
  return ["r", reviewed && "reviewed", level && `lv-${level}`, selected && "sel", quiet && "quiet"].filter(Boolean).join(" ");
}
function renderList() {
  const pane = $("#list");
  const r = st.review;
  const scope = [
    `${r.view.length} hunk${r.view.length === 1 ? "" : "s"}`,
    r.mode === "ledger" && "by symbol",
    st.sinceOnly && "changed since your last look",
    r.path_filter
  ].filter(Boolean);
  replace(
    pane.querySelector(".head"),
    h("span", { class: "t" }, "Reading order"),
    h("span", { class: "sub" }, scope.join(", ")),
    h("span", { class: "grow" }),
    zoomButton("list")
  );
  const body = pane.querySelector(".body");
  const rows = h("div", { class: "rows" });
  const dots = /* @__PURE__ */ new Map();
  let selEl = null;
  for (const row of shownRows()) {
    if (row.kind === "file") {
      rows.append(h("div", { class: "fileband" }, h("span", null, row.path), h("span", null, row.n)));
    } else if (row.kind === "gap") {
      rows.append(h("div", { class: "gap" }));
    } else {
      const items = row.kind === "item" ? [row.i] : row.items;
      const first = r.items[items[0]];
      const selected = items.includes(st.sel);
      const folded = row.kind === "folded";
      const level = LEVELS.find((l) => items.some((i) => r.items[i].level === l));
      const el = h(
        "div",
        {
          class: `${rowClass(items, selected)}${folded ? " folded" : ""}`,
          title: folded ? first.symbol ?? "" : first.rationale,
          onclick: () => {
            st.focus = "list";
            select(items.includes(st.sel) ? st.sel : items[0]);
          }
        },
        h("span", { class: "txt" }, folded ? `${items.length} hunk${items.length === 1 ? "" : "s"}, folded` : sentence(first.rationale)),
        h(
          "span",
          { class: "meta" },
          first.delta && h("span", { class: "since", title: SINCE[first.delta][1] }, SINCE[first.delta][0]),
          level && h("span", { class: `dot ${level}`, title: `${level} finding` })
        )
      );
      for (const i of items) dots.set(i, el);
      if (selected) selEl = el;
      rows.append(el);
    }
  }
  replace(body, rows);
  drawArcs(rows, dots);
  if (selEl) {
    const el = selEl;
    body.scrollTop = centerScroll(el.offsetTop, el.offsetHeight, body.clientHeight, rows.offsetHeight);
  }
}
function drawArcs(rows, dots) {
  const it = st.review.items[st.sel];
  const from = dots.get(st.sel);
  if (!it || !from) return;
  const rail = parseFloat(getComputedStyle(rows).getPropertyValue("--rail")) || 30;
  const mid = (el) => el.offsetTop + el.offsetHeight / 2;
  const y0 = mid(from);
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "arcs");
  svg.setAttribute("height", String(rows.offsetHeight));
  svg.setAttribute("aria-hidden", "true");
  const arcs = /* @__PURE__ */ new Map();
  for (const e of it.edges) {
    const to = e.target === null ? void 0 : dots.get(e.target);
    if (to && to !== from) arcs.set(to, (arcs.get(to) ?? false) || e.needs);
  }
  const lanes = [...arcs].sort(([a], [b]) => Math.abs(mid(a) - y0) - Math.abs(mid(b) - y0));
  const step2 = Math.max(2, (rail - 10) / Math.max(1, lanes.length));
  lanes.forEach(([to, needs], k) => {
    const y1 = mid(to);
    const bulge = Math.min(rail - 4, 7 + k * step2);
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", `M ${rail} ${y0} C ${rail - bulge * 1.3} ${y0}, ${rail - bulge * 1.3} ${y1}, ${rail} ${y1}`);
    path.setAttribute("class", needs ? "needs" : "used");
    svg.append(path);
  });
  rows.prepend(svg);
}
function rowHeight() {
  return document.querySelector("#code .l")?.offsetHeight || 26;
}
function scrollCodeTo(row, force) {
  if (!force) return;
  const body = $("#code .body");
  const el = body.querySelector(`.l[data-k="${Math.max(0, row - 3)}"]`);
  body.scrollTop = el ? el.offsetTop : 0;
}
function followCursor() {
  const body = $("#code .body");
  const rh = rowHeight();
  const el = body.querySelector(".l.cur");
  const top = el ? el.offsetTop : st.cursor * rh;
  if (top < body.scrollTop) body.scrollTop = top;
  else if (top + rh > body.scrollTop + body.clientHeight) body.scrollTop = top + rh - body.clientHeight;
}
function lineEl(row, k, mine, findings, uses, commented) {
  const search = st.search;
  const caret = k === st.cursor ? caretCol(row.text) : -1;
  let at = 0;
  const parts = runs(row.text, row.spans, row.marks, row.kind === "edit" ? row.gone : []).flatMap((run2) => {
    if (run2.gone) return [h("del", { class: "gone", title: "removed" }, run2.text)];
    const piece = (text, extra) => h(run2.changed ? "mark" : "span", { class: [run2.role && `s-${run2.role}`, extra].filter(Boolean).join(" ") || null }, text);
    const cs = [...run2.text];
    const i = caret - at;
    at += cs.length;
    if (i < 0 || i >= cs.length) return [piece(run2.text)];
    const before = cs.slice(0, i).join("");
    const after = cs.slice(i + 1).join("");
    return [before && piece(before), piece(cs[i], "caret"), after && piece(after)].filter((x) => !!x);
  });
  const content = search && search.hits.includes(k) ? highlightHits(row.text, search, k) : parts;
  const n = row.kind === "del" ? "" : row.n;
  const cls = [
    "l",
    row.kind,
    row.hunk !== null && !mine && "other",
    k === st.cursor && "cur",
    st.from !== null && k >= Math.min(st.from, st.cursor) && k <= Math.max(st.from, st.cursor) && "picked",
    row.kind !== "del" && commented.has(row.n) && "commented"
  ].filter(Boolean).join(" ");
  const out = [
    h(
      "div",
      {
        class: cls,
        dataset: { k: String(k) },
        onclick: (e) => {
          st.cursor = k;
          st.col = caretColumn(e, row.text);
          st.focus = "code";
          if (row.hunk !== null && row.hunk !== st.sel) select(row.hunk, { keepCursor: true, focus: "code" });
          else renderCode();
        }
      },
      h("span", { class: "g" }, row.kind !== "del" && uses.has(row.n) ? "\u25B8" : ""),
      h("span", { class: "ln" }, n),
      h("span", { class: "op" }, OP[row.kind]),
      h("span", null, content)
    )
  ];
  if (row.kind !== "del") {
    for (const f of findings.get(row.n) ?? []) {
      out.push(h("div", { class: `fl ${f.level}` }, h("b", null, `${f.level} ${f.name}`), f.message.split("\n")[0]));
    }
  }
  return out;
}
function highlightHits(text, s, k) {
  const out = [];
  const low = text.toLowerCase();
  const q = s.query.toLowerCase();
  let at = 0;
  for (let p = low.indexOf(q); q && p >= 0; p = low.indexOf(q, p + q.length)) {
    out.push(document.createTextNode(text.slice(at, p)));
    out.push(h("span", { class: `hit${s.hits[s.k] === k ? " now" : ""}` }, text.slice(p, p + q.length)));
    at = p + q.length;
  }
  out.push(document.createTextNode(text.slice(at)));
  return out;
}
function renderReading(it) {
  const r = st.review;
  const head = $("#code > .head");
  const actions = $("#code .foot .actions");
  if (!it) {
    replace(head);
    replace(actions);
    return;
  }
  const pos = r.view.indexOf(st.sel);
  const what = it.what.length ? it.what : ["a change ordo found nothing to say about"];
  const search = st.search;
  replace(
    head,
    h(
      "div",
      { class: "kicker" },
      h("span", null, pos >= 0 ? `Hunk ${pos + 1} of ${r.view.length}` : "Outside the current filter"),
      h("span", { class: "path" }, `${it.path}:${it.new[0]}`),
      h("span", { class: "grow" }),
      zoomButton("code")
    ),
    h("h1", { class: "headline" }, prose(sentence(what[0]))),
    (what.length > 1 || it.details.length > 0 || it.grouped_by) && h(
      "div",
      { class: "lede" },
      what.slice(1).map((w) => h("p", null, prose(sentence(w)))),
      it.details.map((d) => h("p", { class: "detail" }, prose(sentence(d)))),
      it.grouped_by && h("p", null, "Grouped by ", prose(it.grouped_by))
    ),
    h(
      "div",
      { class: "pills" },
      it.reviewed && h("span", { class: "pill done" }, "Read"),
      it.delta && h("span", { class: "pill accent" }, SINCE[it.delta][0], sentence(SINCE[it.delta][1].split(" \u2014")[0])),
      it.noise && !/formatting/.test(it.rationale) && h("span", { class: "pill" }, "Quiet: generated or formatting"),
      it.wave !== null && h("span", { class: "pill" }, `Wave ${it.wave}`)
    ),
    (search || st.from !== null) && h(
      "div",
      { class: "status" },
      search && h("span", null, `Searching for \u201C${search.query}\u201D: ${search.hits.length ? `${search.k + 1} of ${search.hits.length}` : "no match"}`),
      st.from !== null && h("span", null, "Selecting lines; press c to comment on them")
    )
  );
  const key = st.keymap.keysFor("toggle-reviewed")[0];
  const s = stops();
  const at = s.indexOf(st.sel);
  const next = at >= 0 && at + 1 < s.length ? r.items[s[at + 1]] : null;
  replace(
    actions,
    h(
      "button",
      { class: `read${it.reviewed ? " done" : ""}`, onclick: () => run("toggle-reviewed"), title: it.reviewed ? "Mark as unread" : "Mark as read and go to the next hunk" },
      it.reviewed ? "Marked as read" : "Mark as read",
      key && h("kbd", null, key)
    ),
    next && h(
      "button",
      { class: "next", onclick: () => step(1), title: `Next: ${next.rationale}`, "aria-label": `next hunk: ${next.rationale}` },
      h("b", { "aria-hidden": "true" }, "\u27F6"),
      h("span", { class: "said" }, prose(sentence(next.rationale)))
    )
  );
}
function sentence(s) {
  return /^[a-z]/.test(s) && !NAME_AT_START.test(s) ? s[0].toUpperCase() + s.slice(1) : s;
}
var NAME = /`[^`]+`|[A-Za-z_][\w]*(?:[._/:][\w]+)+|\b[a-z]+[A-Z]\w*|\b[A-Z][a-z0-9]+[A-Z]\w*|\b\w+\(\)/g;
var NAME_AT_START = /^(?:[A-Za-z_][\w]*(?:[._/:][\w]+)+|[a-z]+[A-Z]\w*)/;
function prose(text) {
  const out = [];
  let at = 0;
  for (const m of text.matchAll(NAME)) {
    if (m.index > at) out.push(text.slice(at, m.index));
    out.push(h("span", { class: "code-word" }, m[0].replace(/^`|`$/g, "")));
    at = m.index + m[0].length;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}
function renderCode() {
  const pane = $("#code");
  const r = st.review;
  const it = r.items[st.sel];
  renderReading(it);
  const body = pane.querySelector(".body");
  if (!st.file || !it) return replace(body);
  const findings = /* @__PURE__ */ new Map();
  for (const f of it.findings) {
    if (f.row == null) continue;
    findings.set(f.row, [...findings.get(f.row) ?? [], f]);
  }
  const uses = new Set(it.uses_at.flatMap((u) => u.rows));
  const commented = new Set(st.file.comments.flatMap((c) => range(c.start, c.end)));
  const code = h("div", { class: "code" });
  st.rows.forEach((row, k) => {
    for (const el of lineEl(row, k, row.hunk === st.sel, findings, uses, commented)) code.append(el);
    if (row.kind !== "del") {
      for (const c of st.file.comments.filter((c2) => c2.end === row.n)) {
        code.append(h("div", { class: "cmt" }, `\u270E ${c.text}${c.stale ? " (lines changed since)" : ""}`));
      }
    }
  });
  const top = body.scrollTop;
  replace(body, code);
  body.scrollTop = top;
}
function caretColumn(e, text) {
  const doc = document;
  const pos = doc.caretPositionFromPoint?.(e.clientX, e.clientY);
  const line = e.currentTarget.lastElementChild;
  if (!pos || !line || !line.contains(pos.offsetNode)) return null;
  const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
  let at = 0;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.parentElement?.closest(".gone")) {
      if (n === pos.offsetNode) return Math.min(at, [...text].length);
      continue;
    }
    if (n === pos.offsetNode) return Math.min(at + [...(n.textContent ?? "").slice(0, pos.offset)].length, [...text].length);
    at += [...n.textContent ?? ""].length;
  }
  return null;
}
function caretCol(text) {
  if (st.col === null) return -1;
  return Math.min(st.col, Math.max(0, [...text].length - 1));
}
function firstWordCol(text) {
  const m = /[A-Za-z_]/.exec(text);
  return m ? [...text.slice(0, m.index)].length : 0;
}
function openEditor() {
  const r = st.review;
  const it = r.items[st.sel];
  if (!it) return;
  const row = st.rows[st.cursor];
  const line = row && row.kind !== "del" ? row.n : it.new[0];
  if (r.editor === "server") {
    act({ op: "open-editor" });
    return;
  }
  const segments = (p) => p.split("/").map(encodeURIComponent).join("/");
  const url = (r.editor || "vscode://file/{root}/{path}:{line}").replaceAll("{root}", segments(r.root).replace(/^\//, "")).replaceAll("{path}", segments(it.path)).replaceAll("{line}", String(line));
  window.open(url, "_self");
}
function range(a, b) {
  return Array.from({ length: Math.max(0, b - a + 1) }, (_, k) => a + k);
}
function moveCursor(by) {
  cursorToRow(Math.max(0, Math.min(st.rows.length - 1, st.cursor + by)));
}
function cursorToRow(k) {
  st.cursor = k;
  const hunk = st.rows[st.cursor]?.hunk;
  if (hunk !== null && hunk !== void 0 && hunk !== st.sel) {
    select(hunk, { keepCursor: true }).then(followCursor);
    return;
  }
  renderCode();
  followCursor();
}
function cursorTo(pred, forward) {
  const n = st.rows.length;
  for (let d = 1; d < n; d++) {
    const k = forward ? st.cursor + d : st.cursor - d;
    if (k < 0 || k >= n) break;
    if (pred(st.rows[k], k)) {
      st.cursor = k;
      renderCode();
      followCursor();
      return;
    }
  }
}
function edgeRows(targets, limit) {
  const r = st.review;
  return targets.slice(0, limit).map(({ target, label }) => {
    if (target === null || !r.items[target]) {
      return h("div", { class: "link out" }, h("span", { class: "who" }, label), h("span", { class: "what" }, "Not part of this review"));
    }
    const t = r.items[target];
    const link = st.whyLinks.length;
    st.whyLinks.push(target);
    return h(
      "div",
      {
        class: `link${st.whyCur === link ? " cur" : ""}`,
        title: "Go there (Enter); come back with C-o",
        onclick: () => jumpTo(target)
      },
      h("span", { class: "who" }, t.symbols[0] ?? t.enclosing ?? t.symbol ?? label ?? basename(t.path)),
      h("span", { class: "loc" }, `${basename(t.path)}:${t.new[0]}`),
      h("span", { class: "what" }, prose(sentence(t.rationale)))
    );
  });
}
function basename(p) {
  return p.slice(p.lastIndexOf("/") + 1);
}
function table(title, hint, targets, empty) {
  const limit = tableLimit(targets.length, st.zoom && st.focus === "why");
  const more = targets.length - limit;
  return [
    h("h3", null, title, hint && h("small", null, hint)),
    targets.length ? h("div", { class: "links" }, edgeRows(targets, limit)) : h("p", { class: "muted" }, empty),
    more > 0 && h(
      "button",
      {
        class: "more",
        onclick: () => {
          st.focus = "why";
          st.zoom = true;
          renderAll();
        }
      },
      `Show ${more} more`
    )
  ];
}
function rulesChecked(r) {
  const parts = [];
  parts.push(r.rules.catalog ? "the construct catalog" : "no catalog (switched off)");
  if (r.rules.own) parts.push(`${r.rules.own} rule${r.rules.own === 1 ? "" : "s"} of your own`);
  if (r.rules.includes.length) parts.push(`rulesets ${r.rules.includes.join(", ")}`);
  return parts.join(", ");
}
function renderWhy() {
  const pane = $("#why");
  const r = st.review;
  const it = r.items[st.sel];
  replace(pane.querySelector(".head"), h("span", { class: "t" }, "Why it is here"), h("span", { class: "grow" }), zoomButton("why"));
  const body = pane.querySelector(".body");
  if (!it) return replace(body);
  st.whyLinks = [];
  const facts = st.facts && st.facts.i === st.sel ? st.facts : null;
  const needs = it.edges.filter((e) => e.needs);
  const neededBy = it.edges.filter((e) => !e.needs);
  const findings = [...it.findings].sort((a, b) => LEVELS.indexOf(a.level) - LEVELS.indexOf(b.level));
  const churn = r.file_churn[it.path];
  const lc = facts?.line_churn;
  const top = body.scrollTop;
  replace(
    body,
    h(
      "div",
      { class: "why" },
      it.note && [h("h3", null, "Your note"), h("p", { class: "warnline" }, it.note)],
      !!facts?.comments.length && [
        h("h3", null, "Your comments"),
        facts.comments.map((c) => h("p", { class: "warnline" }, `Line ${c.start}${c.end > c.start ? `\u2013${c.end}` : ""}: ${c.text}${c.stale ? " (the lines changed since)" : ""}`))
      ],
      !!facts?.out_of_order.length && [
        h("h3", null, "Read too early", h("small", null, "it depends on hunks not read yet")),
        h("div", { class: "links" }, edgeRows(facts.out_of_order.map((t) => ({ target: t, label: "" })), facts.out_of_order.length))
      ],
      table("Needs", "what this hunk uses", needs, "Nothing in this change."),
      table("Needed by", "what uses this hunk", neededBy, "Nothing in this change."),
      facts && table(
        "If you reject it",
        "what loses its footing, in reading order",
        facts.cascade.map((t) => ({ target: t, label: "" })),
        "Rejecting it strands nothing."
      ),
      h("h3", null, "Findings", findings.length > 0 && h("small", null, "the most pressing first")),
      findings.length ? findings.map(
        (f) => h(
          "div",
          { class: "finding" },
          h(
            "div",
            { class: "h" },
            h("span", { class: `lv ${f.level}` }, sentence(f.level)),
            h("span", { class: "name" }, f.name),
            f.row != null && h("button", { class: "at", onclick: () => cursorToLine(f.row), title: "Put the cursor on this line" }, `line ${f.row}`)
          ),
          h("p", null, f.message),
          h("div", { class: "src" }, f.source + (f.original_level ? `; ${f.original_level} in its rule, changed by rules.toml` : ""))
        )
      ) : h("p", { class: "muted" }, `None. Checked against ${rulesChecked(r)}.`),
      it.executed && [
        h("h3", null, "Test coverage"),
        h(
          "p",
          { class: it.executed[0] === it.executed[1] ? "" : "warnline" },
          it.executed[0] === it.executed[1] ? `All ${it.executed[1]} executable lines here run under a test.` : `${it.executed[1] - it.executed[0]} of ${it.executed[1]} executable lines here run under no test.`
        )
      ],
      (churn || lc) && [
        h("h3", null, "History"),
        churn !== void 0 && churn > 0 && h("p", null, `${churn} commit${churn === 1 ? "" : "s"} touched this file in the last six months.`),
        lc && h(
          "p",
          null,
          `These lines changed ${lc.commits}${lc.capped ? "+" : ""} time${lc.commits === 1 ? "" : "s"} before`,
          lc.last ? `, last by ${lc.last.author} on ${lc.last.date}.` : "."
        )
      ],
      it.notes.length > 0 && [h("h3", null, "Also"), it.notes.map((n) => h("p", null, prose(sentence(n))))],
      it.wave !== null && [
        h("h3", null, "Wave"),
        h("p", null, `Wave ${it.wave}${r.wave_intents[String(it.wave)] ? `, asked for: ${r.wave_intents[String(it.wave)]}` : ""}`)
      ]
    )
  );
  body.scrollTop = top;
}
function cursorToLine(n) {
  const k = st.rows.findIndex((row) => row.kind !== "del" && row.n === n);
  if (k < 0) return;
  st.cursor = k;
  st.focus = "code";
  renderAll();
  followCursor();
}
function jumpTo(target) {
  st.jumps.push({ sel: st.sel, cursor: st.cursor });
  select(target, { focus: "code" });
}
function jumpBack() {
  const j = st.jumps.pop();
  if (!j) return;
  select(j.sel, { focus: "code" }).then(() => {
    st.cursor = Math.min(j.cursor, st.rows.length - 1);
    renderCode();
    followCursor();
  });
}
function renderBand() {
  const band = $("#band");
  const r = st.review;
  const help = h("button", { class: "sig help", onclick: () => showHelp(), title: "Every key and command" }, "Keys");
  if (!r) {
    return replace(
      band,
      h("span", { class: "mark" }, "ordo"),
      h("span", { class: "rev" }, rev),
      h("span", { class: "subject" }, st.status),
      st.empty && h("button", { class: "sig", onclick: () => act({ op: "reload" }) }, "Read again")
    );
  }
  const sinceCount = r.delta.new + r.delta.changed + r.delta.moved;
  const plural = { verdict: ["verdict", "verdicts"], warn: ["warning", "warnings"], note: ["note", "notes"] };
  replace(
    band,
    h("span", { class: "mark" }, "ordo"),
    h("span", { class: "rev", title: "the revision under review" }, r.rev),
    h("span", { class: "subject", title: r.subject ?? "" }, r.subject ?? (r.uncommitted ? "Uncommitted work" : "")),
    r.stale && h("button", { class: "sig stale", onclick: () => act({ op: "reload" }), title: "Read the working tree again" }, r.stale),
    r.watch !== "off" && h(
      "span",
      { class: "sig", title: r.watch === "auto" ? "--watch=auto: read again once the working tree settles and you pause" : "--watch=hint: says when the working tree has moved on" },
      r.watch === "auto" ? "Watching, reloads" : "Watching"
    ),
    !r.delta.first_run && sinceCount > 0 && h(
      "button",
      {
        class: "sig",
        "aria-pressed": st.sinceOnly ? "true" : "false",
        title: `${r.delta.new} new, ${r.delta.changed} changed, ${r.delta.moved} moved, ${r.delta.gone} gone. Show only these.`,
        onclick: () => {
          st.sinceOnly = !st.sinceOnly;
          renderAll();
        }
      },
      h("b", null, sinceCount),
      "changed since your last look"
    ),
    LEVELS.filter((l) => r.levels[l]).map(
      (l) => h(
        "button",
        { class: "sig", title: `Go to the next ${plural[l][0]}`, onclick: () => act({ op: "action", action: "finding-next" }) },
        h("span", { class: `dot ${l}` }),
        h("b", null, r.levels[l]),
        plural[l][r.levels[l] === 1 ? 0 : 1]
      )
    ),
    h(
      "span",
      { class: "sig", title: `${r.coverage.both} of ${r.coverage.edges} dependency links have both ends read` },
      h("span", { class: "meter" }, h("i", { style: `width:${r.coverage.total ? 100 * r.coverage.done / r.coverage.total : 0}%` })),
      h("b", null, r.coverage.done),
      `of ${r.coverage.total} read`
    ),
    help
  );
}
var HINTS = {
  list: [["next", "next hunk"], ["finding-next", "next finding"], ["command", "commands"]],
  code: [["next", "next line"], ["search", "search"], ["select-lines", "select"], ["jump-back", "back"]],
  why: [["next", "next link"], ["jump-to-edge", "go there"], ["jump-back", "back"]]
};
function renderHints() {
  const items = HINTS[st.focus].map(([action, label]) => {
    const k = st.keymap.keysFor(action)[0];
    return k ? h("span", null, h("kbd", null, k), label) : null;
  });
  replace($("#hints"), st.notice ? h("span", { class: "notice" }, st.notice) : items);
}
function zoomButton(p) {
  const on = st.zoom && st.focus === p;
  return h(
    "button",
    {
      class: "zoomer",
      title: on ? "Show all three columns" : "Give this column the whole page",
      "aria-label": on ? "restore the columns" : "maximise this column",
      onclick: () => {
        st.zoom = !on;
        st.focus = p;
        renderAll();
      }
    },
    on ? "\u2921" : "\u2922"
  );
}
function showSheet(title, text) {
  const ov = $("#overlay");
  ov.hidden = false;
  replace(
    ov,
    h(
      "div",
      { class: "sheet", role: "dialog", "aria-label": title },
      h("header", null, title, h("button", { onclick: closeSheet, "aria-label": "close" }, "\u2715")),
      typeof text === "string" ? h("pre", null, text) : h("div", { class: "scroll" }, text)
    )
  );
}
function dismissSheet() {
  if ($("#overlay").dataset.kind === "config") {
    act({ op: "config", do: "close" });
    return true;
  }
  return closeSheet();
}
function closeSheet() {
  const ov = $("#overlay");
  if (ov.hidden) return false;
  ov.hidden = true;
  delete ov.dataset.kind;
  canvas = null;
  replace(ov);
  return true;
}
function showHelp() {
  const r = st.review;
  if (!r) return;
  const by = /* @__PURE__ */ new Map();
  for (const b of r.keys) {
    const k = b.prefix ? `${b.prefix} ${b.key}` : b.key;
    by.set(b.action, [...by.get(b.action) ?? [], k]);
  }
  const rows = [...by].map(([a, ks]) => h("tr", null, h("td", null, ks.join(", ")), h("td", null, a)));
  const cmds = [...r.commands, ...CLIENT_COMMANDS].map((c) => h("tr", null, h("td", null, `:${c.name} ${c.args}`), h("td", null, c.help)));
  showSheet("keys and commands", h("div", { class: "why" }, h("table", null, h("tbody", null, rows)), h("h3", null, "commands"), h("table", null, h("tbody", null, cmds))));
}
var toastTimer = 0;
function toast(msg, err = false) {
  const t = $("#toast");
  t.textContent = msg;
  t.className = `show${err ? " err" : ""}`;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.className = "", err ? 4e3 : 2200);
}
function renderAll() {
  document.body.classList.toggle("zoom", st.zoom);
  for (const p of PANES) {
    const el = $(`#${p}`);
    el.classList.toggle("focus", st.focus === p);
    el.classList.toggle("zoomed", st.zoom && st.focus === p);
  }
  renderBand();
  if (!st.review) {
    for (const p of PANES) {
      replace($(`#${p} > .head`));
      replace($(`#${p} > .body`));
    }
    replace($("#code .foot .actions"));
    replace($("#code > .body"), h("div", { class: "loading" }, st.empty ? "Nothing to read in this revision." : "Reading the change\u2026", h("small", null, st.status)));
    renderHints();
    return;
  }
  renderList();
  renderCode();
  renderWhy();
  renderHints();
}
var SIZES = { small: 0.9, normal: 1, large: 1.15, larger: 1.3 };
function sizeOf(arg) {
  const a = arg.trim();
  if (!a) return 1;
  if (a in SIZES) return SIZES[a];
  const n = a.endsWith("%") ? parseFloat(a) / 100 : parseFloat(a);
  return Number.isFinite(n) && n >= 0.6 && n <= 2.5 ? n : null;
}
function applySize(ui) {
  document.documentElement.style.setProperty("--ui", String(ui));
}
var paletteMode = "command";
var paletteCur = 0;
var palettePicked = false;
function openPalette(mode, text = "") {
  paletteMode = mode;
  const form = $("#palette");
  form.hidden = false;
  $("#palette label").textContent = mode === "command" ? ":" : "/";
  const input = $("#palette-input");
  input.value = text;
  paletteCur = 0;
  palettePicked = false;
  renderPalette();
  input.focus();
}
function closePalette() {
  $("#palette").hidden = true;
  document.activeElement?.blur();
}
function argCandidates() {
  const r = st.review;
  return {
    strategy: ["comprehension", "defs-first", "file"],
    mode: ["ledger", "hunks"],
    watch: ["on", "off", "auto"],
    "only-wave": ["last", "all", ...Object.keys(r.wave_intents)],
    yank: ["all"],
    send: ["all"],
    theme: ["auto", ...themeNames()],
    size: Object.keys(SIZES),
    goto: [...new Set(r.items.map((i) => i.path))],
    filter: [...new Set(r.items.map((i) => i.path.replace(/[^/]*$/, "*")))]
  };
}
function candidates() {
  if (paletteMode !== "command" || !st.review) return [];
  const input = $("#palette-input").value;
  return complete(input, [...st.review.commands, ...CLIENT_COMMANDS], argCandidates()).slice(0, 40);
}
function renderPalette() {
  const list = $("#palette-list");
  if (paletteMode !== "command" || !st.review) return replace(list);
  const all = [...st.review.commands, ...CLIENT_COMMANDS];
  replace(
    list,
    candidates().map((c, k) => {
      const cmd = all.find((x) => x.name === c.split(" ")[0]);
      return h(
        "li",
        {
          class: k === paletteCur ? "cur" : "",
          onmousedown: (e) => {
            e.preventDefault();
            $("#palette-input").value = c;
            runPalette();
          }
        },
        h("b", null, c),
        h("span", null, cmd && !c.includes(" ") ? cmd.help : "")
      );
    })
  );
}
async function runPalette() {
  const picked = palettePicked && candidates()[paletteCur];
  const value = picked || $("#palette-input").value;
  closePalette();
  if (paletteMode === "search") return runSearch(value);
  const { name, arg } = parseCommand(value);
  if (!name) return;
  if (name === "theme") {
    applyTheme(arg || "auto");
    localStorage.setItem("ordo.look", arg || "auto");
    return;
  }
  if (name === "size") {
    const ui = sizeOf(arg);
    if (ui === null) return toast(`:size takes ${Object.keys(SIZES).join(", ")} or a factor like 1.2`, true);
    applySize(ui);
    localStorage.setItem("ordo.size", String(ui));
    return renderAll();
  }
  const out = await act({ op: "command", line_text: value });
  if (out && !out.error && name === "comment" && st.from !== null) {
    st.from = null;
    renderCode();
  }
}
function runSearch(query, forward = true) {
  if (!query) {
    st.search = null;
    return renderCode();
  }
  const q = query.toLowerCase();
  const hits = st.rows.flatMap((row, k2) => row.text.toLowerCase().includes(q) ? [k2] : []);
  const after = hits.findIndex((x) => x > st.cursor);
  const at = hits.indexOf(st.cursor);
  const k = forward ? Math.max(0, at >= 0 ? at : after) : Math.max(0, (after < 0 ? hits.length : after) - (at >= 0 ? 2 : 1));
  st.search = { query, hits, k };
  if (hits.length) st.cursor = hits[k];
  st.focus = "code";
  renderAll();
  followCursor();
}
function searchStep(by) {
  const s = st.search;
  if (!s || !s.hits.length) return;
  s.k = (s.k + by + s.hits.length) % s.hits.length;
  st.cursor = s.hits[s.k];
  renderCode();
  followCursor();
}
function page(by) {
  const body = $(`#${st.focus} .body`);
  body.scrollTop += by * body.clientHeight;
}
function run(action) {
  const r = st.review;
  if (!r) return;
  const focus = st.focus;
  switch (action) {
    case "quit":
      if (dismissSheet()) return;
      if (st.search) st.search = null;
      else if (st.from !== null) st.from = null;
      else if (st.zoom) st.zoom = false;
      return renderAll();
    case "help":
      return showHelp();
    case "command":
      return openPalette("command");
    case "command-goto":
      return openPalette("command", "goto ");
    case "search":
      return openPalette("search");
    case "search-next":
      return searchStep(1);
    case "search-prev":
      return searchStep(-1);
    case "symbol-next":
    case "symbol-prev": {
      const it = r.items[st.sel];
      const name = it?.symbols[0] ?? it?.enclosing;
      const by = action === "symbol-next" ? 1 : -1;
      if (name && st.search?.query === name) return searchStep(by);
      if (name) runSearch(name, by > 0);
      return;
    }
    case "next":
    case "prev": {
      const by = action === "next" ? 1 : -1;
      if (focus === "list") return step(by);
      if (focus === "code") return moveCursor(by);
      return whyStep(by);
    }
    case "first":
      if (focus === "list") return void (stops().length && select(stops()[0]));
      if (focus === "code") return moveCursor(-st.rows.length);
      $("#why .body").scrollTop = 0;
      return;
    case "last":
      if (focus === "list") return void (stops().length && select(stops().at(-1)));
      if (focus === "code") return moveCursor(st.rows.length);
      $("#why .body").scrollTop = 1e9;
      return;
    case "page-down":
      return page(1);
    case "page-up":
      return page(-1);
    case "half-down":
      return page(0.5);
    case "half-up":
      return page(-0.5);
    case "toggle-reviewed": {
      const s = stops();
      const at = s.indexOf(st.sel);
      act({ op: "mark" }).then((out) => {
        if (out && !out.error && at >= 0 && at + 1 < s.length) select(s[at + 1]);
      });
      return;
    }
    case "focus-next":
    case "focus-prev": {
      const k = PANES.indexOf(focus) + (action === "focus-next" ? 1 : PANES.length - 1);
      st.focus = PANES[k % PANES.length];
      return renderAll();
    }
    case "focus-list":
    case "focus-code":
    case "focus-why": {
      const p = action.slice(6);
      if (st.focus === p) st.zoom = !st.zoom;
      st.focus = p;
      return renderAll();
    }
    case "zoom":
      st.zoom = !st.zoom;
      return renderAll();
    case "finding-next":
    case "finding-prev":
    case "fold-toggle":
    case "fold-open":
    case "fold-close":
    case "fold-open-all":
    case "fold-close-all":
    case "wave-next":
    case "wave-prev":
      act({ op: "action", action });
      return;
    case "jump-to-edge":
      if (focus === "why" && st.whyLinks[st.whyCur] !== void 0) jumpTo(st.whyLinks[st.whyCur]);
      return;
    case "jump-back":
      return jumpBack();
    case "mark-next":
    case "mark-prev": {
      const uses = new Set(r.items[st.sel].uses_at.flatMap((u) => u.rows));
      return cursorTo((row) => row.kind !== "del" && uses.has(row.n), action === "mark-next");
    }
    case "comment-next":
    case "comment-prev": {
      const c = new Set(st.file?.comments.map((c2) => c2.start) ?? []);
      return cursorTo((row) => row.kind !== "del" && c.has(row.n), action === "comment-next");
    }
    case "para-next":
    case "para-prev":
      return cursorTo((row) => row.text.trim() === "", action === "para-next");
    case "cursor-left":
    case "cursor-right":
    case "line-start":
    case "line-end": {
      const text = st.rows[st.cursor]?.text ?? "";
      const last = Math.max(0, [...text].length - 1);
      const now = st.col === null ? firstWordCol(text) : caretCol(text);
      st.col = action === "line-start" ? 0 : action === "line-end" ? last : Math.max(0, Math.min(last, now + (action === "cursor-right" ? 1 : -1)));
      st.focus = "code";
      return renderAll();
    }
    case "word-next":
    case "word-prev":
    case "word-end": {
      const kind = action === "word-next" ? "next" : action === "word-prev" ? "prev" : "end";
      const text = st.rows[st.cursor]?.text ?? "";
      const to = wordMove(
        st.rows.map((r2) => r2.text),
        { row: st.cursor, col: st.col === null ? firstWordCol(text) : caretCol(text) },
        kind
      );
      st.col = to.col;
      st.focus = "code";
      if (to.row === st.cursor) return renderAll();
      return cursorToRow(to.row);
    }
    case "select-lines":
      st.from = st.from === null ? st.cursor : null;
      st.focus = "code";
      return renderAll();
    case "comment":
      return openPalette("command", "comment ");
    case "reload":
      act({ op: "reload" });
      return;
    case "hover":
      if (focus === "why" && st.whyLinks[st.whyCur] !== void 0) return void previewHunk(st.whyLinks[st.whyCur]);
      act({ op: "hover", col: st.col === null ? firstWordCol(st.rows[st.cursor]?.text ?? "") : caretCol(st.rows[st.cursor]?.text ?? "") });
      return;
    case "open-editor":
      return openEditor();
    case "open-deps":
    case "focus-deps":
      return void openCanvas();
  }
}
function whyStep(by) {
  const n = st.whyLinks.length;
  if (!n) {
    $("#why .body").scrollTop += by * 40;
    return;
  }
  st.whyCur = Math.max(0, Math.min(n - 1, st.whyCur + by));
  renderWhy();
  $("#why tr.cur")?.scrollIntoView({ block: "nearest" });
}
function onKey(e) {
  const t = e.target;
  if (t && (t.tagName === "INPUT" || t.tagName === "SELECT" || t.tagName === "TEXTAREA") && t.id !== "palette-input") return;
  if (t?.tagName === "BUTTON" && (e.key === "Enter" || e.key === " ")) return;
  const paletteOpen = !$("#palette").hidden;
  if (paletteOpen) return paletteKey(e);
  const key = keyLabel(e);
  if (!key) return;
  if (key === "Enter" && canvas) {
    e.preventDefault();
    return void canvasKey("jump-to-edge");
  }
  if (key === "Esc" && $("#overlay").dataset.kind === "config") {
    e.preventDefault();
    return void dismissSheet();
  }
  if (key === "Enter" && st.focus === "why" && st.whyCur >= 0) {
    e.preventDefault();
    return run("jump-to-edge");
  }
  const pending = st.pending;
  const res = st.keymap.resolve(pending, key);
  clearTimeout(st.pendingTimer);
  st.pending = null;
  if ("pending" in res) {
    e.preventDefault();
    st.pending = key;
    const bare = st.keymap.bare(key);
    if (bare) st.pendingTimer = window.setTimeout(() => (st.pending = null, run(bare)), CHORD_TIMEOUT);
    return;
  }
  if ("act" in res) {
    e.preventDefault();
    if (canvasKey(res.act)) return;
    return run(res.act);
  }
  if (pending) {
    const bare = st.keymap.bare(pending);
    if (bare) run(bare);
    onKey(e);
  }
}
function paletteKey(e) {
  const c = candidates();
  if (e.key === "Escape") {
    e.preventDefault();
    return closePalette();
  }
  if (e.key === "Tab" || e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if (!c.length) return;
    if (e.key === "Tab") {
      $("#palette-input").value = c[paletteCur] + (c[paletteCur].includes(" ") ? "" : " ");
      paletteCur = 0;
    } else {
      paletteCur = (paletteCur + (e.key === "ArrowDown" ? 1 : c.length - 1)) % c.length;
      palettePicked = true;
    }
    return renderPalette();
  }
}
function excerpt(f, hunk, context, max) {
  if (f.missing) return h("p", { class: "muted" }, "no text to show for this file");
  const rows = codeRows(f);
  const ks = rows.flatMap((r, k) => r.hunk === hunk ? [k] : []);
  const from = Math.max(0, (ks[0] ?? 0) - context);
  const to = Math.min(rows.length, Math.min((ks.at(-1) ?? 0) + context + 1, from + max));
  return h(
    "div",
    { class: "code" },
    rows.slice(from, to).map(
      (row) => h(
        "div",
        { class: `l ${row.kind}${row.hunk !== null && row.hunk !== hunk ? " other" : ""}` },
        h("span", { class: "g" }),
        h("span", { class: "ln" }, row.kind === "del" ? "" : row.n),
        h("span", { class: "op" }, OP[row.kind]),
        h(
          "span",
          null,
          runs(row.text, row.spans, row.marks, row.kind === "edit" ? row.gone : []).map(
            (run2) => run2.gone ? h("del", { class: "gone" }, run2.text) : h(run2.changed ? "mark" : "span", { class: run2.role ? `s-${run2.role}` : null }, run2.text)
          )
        )
      )
    )
  );
}
function hunkTitle(i) {
  const t = st.review.items[i];
  return `${t.symbols[0] ?? t.enclosing ?? t.symbol ?? basename(t.path)} \xB7 ${basename(t.path)}:${t.new[0]}`;
}
async function previewHunk(i) {
  const t = st.review.items[i];
  const f = await fileOf(t.path);
  showSheet(hunkTitle(i), h("div", null, h("p", { class: "muted" }, t.rationale), h("div", { class: "src" }, excerpt(f, i, 2, 40))));
}
var canvas = null;
async function openCanvas() {
  const r = st.review;
  const it = r.items[st.sel];
  const cards = it.edges.filter((e) => e.target !== null && r.items[e.target]).map((e) => ({ i: e.target, needs: e.needs }));
  if (!cards.length) return toast("nothing in this change needs this hunk, and it needs nothing");
  canvas = { anchor: st.sel, cards, sel: 0 };
  await renderCanvas();
}
async function renderCanvas() {
  if (!canvas) return;
  const c = canvas;
  const files = await Promise.all([c.anchor, ...c.cards.map((k) => k.i)].map((i) => fileOf(st.review.items[i].path)));
  if (canvas !== c) return;
  const card = (k) => {
    const { i } = c.cards[k];
    return h(
      "div",
      { class: `card${k === c.sel ? " cur" : ""}`, onclick: () => goCanvas(k) },
      h("header", null, hunkTitle(i), h("span", { class: "muted" }, st.review.items[i].rationale)),
      excerpt(files[k + 1], i, 1, 14)
    );
  };
  const side = (needs) => h(
    "div",
    { class: "side" },
    h("h3", null, needs ? "needs \u2014 what this hunk uses" : "needed by \u2014 what uses this hunk"),
    c.cards.map((x, k) => x.needs === needs ? card(k) : null)
  );
  showSheet(
    `dependencies of ${hunkTitle(c.anchor)}`,
    h(
      "div",
      { class: "canvas src" },
      h("div", { class: "anchor" }, excerpt(files[0], c.anchor, 1, 16)),
      h("div", { class: "sides" }, side(true), side(false))
    )
  );
  $("#overlay").querySelector(".sheet")?.classList.add("wide");
  $("#overlay .card.cur")?.scrollIntoView({ block: "nearest" });
}
function goCanvas(k) {
  if (!canvas) return;
  const target = canvas.cards[k].i;
  canvas = null;
  closeSheet();
  jumpTo(target);
}
function canvasKey(action) {
  if (!canvas) return false;
  const n = canvas.cards.length;
  if (action === "next" || action === "prev") {
    canvas.sel = (canvas.sel + (action === "next" ? 1 : n - 1)) % n;
    renderCanvas();
  } else if (action === "jump-to-edge") {
    goCanvas(canvas.sel);
  } else if (action === "quit" || action === "open-deps" || action === "focus-deps") {
    canvas = null;
    closeSheet();
  }
  return true;
}
function renderConfig() {
  const cfg = st.review?.config;
  const ov = $("#overlay");
  if (!cfg) {
    if (ov.dataset.kind === "config") closeSheet();
    return;
  }
  const step2 = (body2) => act({ op: "config", ...body2 });
  const field = (f) => {
    let control;
    if (f.kind === "flag") {
      control = h("input", { type: "checkbox", checked: f.value === true, onchange: () => step2({ do: "toggle", row: f.row }) });
    } else if (f.kind === "choice") {
      control = h(
        "button",
        { class: "choice", title: (f.options ?? []).join(" \xB7 "), onclick: () => step2({ do: "toggle", row: f.row }) },
        String(f.value ?? "")
      );
    } else {
      control = h("input", {
        value: String(f.value ?? ""),
        placeholder: "(terminal)",
        onchange: (e) => step2({ do: "edit", row: f.row, text: e.target.value })
      });
    }
    return h("tr", null, h("td", null, f.label), h("td", null, control), h("td", { class: "muted" }, f.help));
  };
  const body = h(
    "div",
    { class: "why config" },
    cfg.sections.map((s) => [h("h3", null, s.title, h("small", null, s.file)), h("table", null, h("tbody", null, s.fields.map(field)))])
  );
  const top = ov.querySelector(".scroll")?.scrollTop ?? 0;
  showSheet(cfg.dirty ? "settings \u2014 changed, not written" : "settings", body);
  ov.dataset.kind = "config";
  const header = ov.querySelector(".sheet > header");
  header.insertBefore(h("button", { class: "write", disabled: !cfg.dirty, onclick: () => step2({ do: "write" }) }, "Write to files"), header.lastChild);
  header.lastElementChild.addEventListener("click", () => step2({ do: "close" }));
  ov.querySelector(".scroll").scrollTop = top;
}
function wireSplits() {
  const panes = $("#panes");
  let saved = null;
  try {
    saved = JSON.parse(localStorage.getItem("ordo.splits") ?? "null");
  } catch {
    localStorage.removeItem("ordo.splits");
  }
  if (saved) {
    panes.style.setProperty("--c0", saved[0]);
    panes.style.setProperty("--c2", saved[1]);
  }
  for (const split of document.querySelectorAll(".split")) {
    split.addEventListener("pointerdown", (e) => {
      split.setPointerCapture(e.pointerId);
      split.classList.add("drag");
      const which = split.dataset.split;
      const move = (ev) => {
        const box = panes.getBoundingClientRect();
        const x = (ev.clientX - box.left) / box.width * 100;
        if (which === "0") panes.style.setProperty("--c0", `${Math.min(60, Math.max(12, x))}%`);
        else panes.style.setProperty("--c2", `${Math.min(60, Math.max(14, 100 - x))}%`);
      };
      const up = () => {
        split.classList.remove("drag");
        split.removeEventListener("pointermove", move);
        localStorage.setItem("ordo.splits", JSON.stringify([panes.style.getPropertyValue("--c0") || "27%", panes.style.getPropertyValue("--c2") || "30%"]));
        if (st.review) renderList();
      };
      split.addEventListener("pointermove", move);
      split.addEventListener("pointerup", up, { once: true });
    });
  }
}
function wire() {
  document.addEventListener("keydown", onKey);
  for (const p of PANES) {
    $(`#${p}`).addEventListener("mousedown", () => {
      if (st.focus !== p) {
        st.focus = p;
        renderHints();
        for (const q of PANES) $(`#${q}`).classList.toggle("focus", q === p);
      }
    });
  }
  const form = $("#palette");
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    runPalette();
  });
  $("#palette-input").addEventListener("input", () => {
    paletteCur = 0;
    palettePicked = false;
    renderPalette();
  });
  $("#palette-input").addEventListener("blur", () => setTimeout(() => form.hidden = true, 100));
  $("#overlay").addEventListener("mousedown", (e) => {
    if (e.target === e.currentTarget) dismissSheet();
  });
  window.addEventListener("resize", () => st.review && renderList());
  wireSplits();
  api.events((event, data) => {
    if (event === "progress" && !st.review) {
      st.status = data.status;
      renderAll();
    } else if (event === "progress") {
      st.notice = data.status;
      renderHints();
    } else if (event === "changed" && data.version !== st.version) {
      st.notice = "";
      refresh();
    }
  });
}
async function boot() {
  wire();
  renderAll();
  applySize(Number(localStorage.getItem("ordo.size")) || 1);
  loadThemes(api).then(() => applyTheme(localStorage.getItem("ordo.look") ?? "auto"));
  await refresh();
}
boot();
