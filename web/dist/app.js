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
    const q2 = new URLSearchParams({ rev: this.rev });
    for (const [k, v] of Object.entries(params)) q2.set(k, String(v));
    return `${path}?${q2}`;
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
  events(on2) {
    const es = new EventSource(this.url("/api/events", { t: this.token }));
    for (const name of ["progress", "changed"]) {
      es.addEventListener(name, (e) => on2(name, JSON.parse(e.data)));
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
    for (const [at2, t] of gone) if (at2 === k) out.push({ text: t, role: null, changed: false, gone: true });
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
function wordMove(lines, at2, kind) {
  const { chars, starts } = flatten(lines);
  if (!chars.length) return { row: 0, col: 0 };
  let i = Math.min((starts[at2.row] ?? 0) + at2.col, chars.length - 1);
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
function wordAt(text, col) {
  const cs = [...text];
  const isWord = (c) => !!c && /\w/.test(c);
  if (!isWord(cs[col])) return null;
  let a = col;
  while (isWord(cs[a - 1])) a--;
  let b = col;
  while (isWord(cs[b + 1])) b++;
  const word = cs.slice(a, b + 1).join("");
  return /^[A-Za-z_]/.test(word) ? { word, start: a } : null;
}
function stripMarks(rows, sel, findings, comments, hits) {
  const out = [];
  const last = /* @__PURE__ */ new Map();
  const add = (kind, k) => {
    const m = last.get(kind);
    if (m && m.to === k - 1) m.to = k;
    else {
      const next = { kind, from: k, to: k };
      out.push(next);
      last.set(kind, next);
    }
  };
  rows.forEach((row, k) => {
    if (row.kind !== "ctx" && row.hunk !== null) add(row.hunk === sel ? "mine" : "other", k);
  });
  rows.forEach((row, k) => {
    if (row.kind === "del") return;
    const level = findings.get(row.n);
    if (level) add(level, k);
    if (comments.has(row.n)) add("comment", k);
  });
  for (const k of hits) add("hit", k);
  return out;
}

// node_modules/@dagrejs/dagre/dist/dagre.esm.js
var Te = Object.defineProperty;
var Sn = (e, n, t) => n in e ? Te(e, n, { enumerable: true, configurable: true, writable: true, value: t }) : e[n] = t;
var Mn = (e, n) => {
  for (var t in n) Te(e, t, { get: n[t], enumerable: true });
};
var je = (e, n, t) => Sn(e, typeof n != "symbol" ? n + "" : n, t);
var ie = {};
Mn(ie, { Graph: () => T, alg: () => H });
var Pn = Object.defineProperty;
var Se = (e, n) => {
  for (var t in n) Pn(e, t, { get: n[t], enumerable: true });
};
var Q = class {
  constructor(e) {
    this._isDirected = true, this._isMultigraph = false, this._isCompound = false, this._nodes = {}, this._in = {}, this._preds = {}, this._out = {}, this._sucs = {}, this._edgeObjs = {}, this._edgeLabels = {}, this._nodeCount = 0, this._edgeCount = 0, this._defaultNodeLabelFn = () => {
    }, this._defaultEdgeLabelFn = () => {
    }, e && (this._isDirected = "directed" in e ? e.directed : true, this._isMultigraph = "multigraph" in e ? e.multigraph : false, this._isCompound = "compound" in e ? e.compound : false), this._isCompound && (this._parent = {}, this._children = {}, this._children["\0"] = {});
  }
  isDirected() {
    return this._isDirected;
  }
  isMultigraph() {
    return this._isMultigraph;
  }
  isCompound() {
    return this._isCompound;
  }
  setGraph(e) {
    return this._label = e, this;
  }
  graph() {
    return this._label;
  }
  setDefaultNodeLabel(e) {
    return typeof e != "function" ? this._defaultNodeLabelFn = () => e : this._defaultNodeLabelFn = e, this;
  }
  nodeCount() {
    return this._nodeCount;
  }
  nodes() {
    return Object.keys(this._nodes);
  }
  sources() {
    return this.nodes().filter((e) => Object.keys(this._in[e]).length === 0);
  }
  sinks() {
    return this.nodes().filter((e) => Object.keys(this._out[e]).length === 0);
  }
  setNodes(e, n) {
    return e.forEach((t) => {
      n !== void 0 ? this.setNode(t, n) : this.setNode(t);
    }), this;
  }
  setNode(e, n) {
    return e in this._nodes ? (arguments.length > 1 && (this._nodes[e] = n), this) : (this._nodes[e] = arguments.length > 1 ? n : this._defaultNodeLabelFn(e), this._isCompound && (this._parent[e] = "\0", this._children[e] = {}, this._children["\0"][e] = true), this._in[e] = {}, this._preds[e] = {}, this._out[e] = {}, this._sucs[e] = {}, ++this._nodeCount, this);
  }
  node(e) {
    return this._nodes[e];
  }
  hasNode(e) {
    return e in this._nodes;
  }
  removeNode(e) {
    if (e in this._nodes) {
      let n = (t) => this.removeEdge(this._edgeObjs[t]);
      delete this._nodes[e], this._isCompound && (this._removeFromParentsChildList(e), delete this._parent[e], this.children(e).forEach((t) => {
        this.setParent(t);
      }), delete this._children[e]), Object.keys(this._in[e]).forEach(n), delete this._in[e], delete this._preds[e], Object.keys(this._out[e]).forEach(n), delete this._out[e], delete this._sucs[e], --this._nodeCount;
    }
    return this;
  }
  setParent(e, n) {
    if (!this._isCompound) throw new Error("Cannot set parent in a non-compound graph");
    if (n === void 0) n = "\0";
    else {
      n += "";
      for (let t = n; t !== void 0; t = this.parent(t)) if (t === e) throw new Error("Setting " + n + " as parent of " + e + " would create a cycle");
      this.setNode(n);
    }
    return this.setNode(e), this._removeFromParentsChildList(e), this._parent[e] = n, this._children[n][e] = true, this;
  }
  parent(e) {
    if (this._isCompound) {
      let n = this._parent[e];
      if (n !== "\0") return n;
    }
  }
  children(e = "\0") {
    if (this._isCompound) {
      let n = this._children[e];
      if (n) return Object.keys(n);
    } else {
      if (e === "\0") return this.nodes();
      if (this.hasNode(e)) return [];
    }
    return [];
  }
  predecessors(e) {
    let n = this._preds[e];
    if (n) return Object.keys(n);
  }
  successors(e) {
    let n = this._sucs[e];
    if (n) return Object.keys(n);
  }
  neighbors(e) {
    let n = this.predecessors(e);
    if (n) {
      let t = new Set(n), r = this.successors(e);
      if (r) for (let o of r) t.add(o);
      return Array.from(t.values());
    }
  }
  isLeaf(e) {
    var n;
    let t;
    return this.isDirected() ? t = this.successors(e) : t = this.neighbors(e), ((n = t == null ? void 0 : t.length) != null ? n : 0) === 0;
  }
  filterNodes(e) {
    let n = new this.constructor({ directed: this._isDirected, multigraph: this._isMultigraph, compound: this._isCompound });
    n.setGraph(this.graph()), Object.entries(this._nodes).forEach(([o, i]) => {
      e(o) && n.setNode(o, i);
    }), Object.values(this._edgeObjs).forEach((o) => {
      n.hasNode(o.v) && n.hasNode(o.w) && n.setEdge(o, this.edge(o));
    });
    let t = {}, r = (o) => {
      let i = this.parent(o);
      return !i || n.hasNode(i) ? (t[o] = i, i) : i in t ? t[i] : r(i);
    };
    return this._isCompound && n.nodes().forEach((o) => n.setParent(o, r(o))), n;
  }
  setDefaultEdgeLabel(e) {
    return typeof e != "function" ? this._defaultEdgeLabelFn = () => e : this._defaultEdgeLabelFn = e, this;
  }
  edgeCount() {
    return this._edgeCount;
  }
  edges() {
    return Object.values(this._edgeObjs);
  }
  setPath(e, n) {
    return e.reduce((t, r) => (n !== void 0 ? this.setEdge(t, r, n) : this.setEdge(t, r), r)), this;
  }
  setEdge(e, n, t, r) {
    let o, i, s, a, l = false;
    typeof e == "object" && e !== null && "v" in e ? (o = e.v, i = e.w, s = e.name, arguments.length === 2 && (a = n, l = true)) : (o = e, i = n, s = r, arguments.length > 2 && (a = t, l = true)), o = "" + o, i = "" + i, s !== void 0 && (s = "" + s);
    let u = z(this._isDirected, o, i, s);
    if (u in this._edgeLabels) return l && (this._edgeLabels[u] = a), this;
    if (s !== void 0 && !this._isMultigraph) throw new Error("Cannot set a named edge when isMultigraph = false");
    this.setNode(o), this.setNode(i), this._edgeLabels[u] = l ? a : this._defaultEdgeLabelFn(o, i, s);
    let d = Fn(this._isDirected, o, i, s);
    return o = d.v, i = d.w, Object.freeze(d), this._edgeObjs[u] = d, Re(this._preds[i], o), Re(this._sucs[o], i), this._in[i][u] = d, this._out[o][u] = d, this._edgeCount++, this;
  }
  edge(e, n, t) {
    let r = arguments.length === 1 ? oe(this._isDirected, e) : z(this._isDirected, e, n, t);
    return this._edgeLabels[r];
  }
  edgeAsObj(e, n, t) {
    let r = arguments.length === 1 ? this.edge(e) : this.edge(e, n, t);
    return typeof r != "object" || r === null ? { label: r } : r;
  }
  hasEdge(e, n, t) {
    return (arguments.length === 1 ? oe(this._isDirected, e) : z(this._isDirected, e, n, t)) in this._edgeLabels;
  }
  removeEdge(e, n, t) {
    let r = arguments.length === 1 ? oe(this._isDirected, e) : z(this._isDirected, e, n, t), o = this._edgeObjs[r];
    if (o) {
      let i = o.v, s = o.w;
      delete this._edgeLabels[r], delete this._edgeObjs[r], Ie(this._preds[s], i), Ie(this._sucs[i], s), delete this._in[s][r], delete this._out[i][r], this._edgeCount--;
    }
    return this;
  }
  inEdges(e, n) {
    return this.isDirected() ? this.filterEdges(this._in[e], e, n) : this.nodeEdges(e, n);
  }
  outEdges(e, n) {
    return this.isDirected() ? this.filterEdges(this._out[e], e, n) : this.nodeEdges(e, n);
  }
  nodeEdges(e, n) {
    if (e in this._nodes) return this.filterEdges({ ...this._in[e], ...this._out[e] }, e, n);
  }
  _removeFromParentsChildList(e) {
    delete this._children[this._parent[e]][e];
  }
  filterEdges(e, n, t) {
    if (!e) return;
    let r = Object.values(e);
    return t ? r.filter((o) => o.v === n && o.w === t || o.v === t && o.w === n) : r;
  }
};
function Re(e, n) {
  e[n] ? e[n]++ : e[n] = 1;
}
function Ie(e, n) {
  e[n] !== void 0 && !--e[n] && delete e[n];
}
function z(e, n, t, r) {
  let o = "" + n, i = "" + t;
  if (!e && o > i) {
    let s = o;
    o = i, i = s;
  }
  return o + "" + i + "" + (r === void 0 ? "\0" : r);
}
function Fn(e, n, t, r) {
  let o = "" + n, i = "" + t;
  if (!e && o > i) {
    let a = o;
    o = i, i = a;
  }
  let s = { v: o, w: i };
  return r && (s.name = r), s;
}
function oe(e, n) {
  return z(e, n.v, n.w, n.name);
}
var Dn = {};
Se(Dn, { read: () => Bn, write: () => An });
function An(e) {
  let n = { options: { directed: e.isDirected(), multigraph: e.isMultigraph(), compound: e.isCompound() }, nodes: Vn(e), edges: Yn(e) }, t = e.graph();
  return t !== void 0 && (n.value = structuredClone(t)), n;
}
function Vn(e) {
  return e.nodes().map((n) => {
    let t = e.node(n), r = e.parent(n), o = { v: n };
    return t !== void 0 && (o.value = t), r !== void 0 && (o.parent = r), o;
  });
}
function Yn(e) {
  return e.edges().map((n) => {
    let t = e.edge(n), r = { v: n.v, w: n.w };
    return n.name !== void 0 && (r.name = n.name), t !== void 0 && (r.value = t), r;
  });
}
function Bn(e) {
  let n = new Q(e.options);
  return e.value !== void 0 && n.setGraph(e.value), e.nodes.forEach((t) => {
    n.setNode(t.v, t.value), t.parent && n.setParent(t.v, t.parent);
  }), e.edges.forEach((t) => {
    n.setEdge({ v: t.v, w: t.w, name: t.name }, t.value);
  }), n;
}
var H = {};
Se(H, { CycleException: () => K, bellmanFord: () => Me, components: () => zn, dijkstra: () => J, dijkstraAll: () => $n, findCycles: () => Un, floydWarshall: () => Kn, isAcyclic: () => Zn, postorder: () => nt, preorder: () => tt, prim: () => rt, shortestPaths: () => ot, tarjan: () => Fe, topsort: () => De });
var Wn = () => 1;
function Me(e, n, t, r) {
  return Xn(e, String(n), t || Wn, r || function(o) {
    var i;
    return (i = e.outEdges(o)) != null ? i : [];
  });
}
function Xn(e, n, t, r) {
  let o = {}, i, s = 0, a = e.nodes(), l = function(c) {
    let f = o[c.v], h2 = o[c.w];
    if (!f || !h2) return;
    let g = t(c);
    f.distance + g < h2.distance && (o[c.w] = { distance: f.distance + g, predecessor: c.v }, i = true);
  }, u = function() {
    a.forEach(function(c) {
      r(c).forEach(function(f) {
        let h2 = f.v === c ? f.v : f.w, g = h2 === f.v ? f.w : f.v;
        l({ v: h2, w: g });
      });
    });
  };
  a.forEach(function(c) {
    let f = c === n ? 0 : Number.POSITIVE_INFINITY;
    o[c] = { distance: f, predecessor: "" };
  });
  let d = a.length;
  for (let c = 1; c < d && (i = false, s++, u(), !!i); c++) ;
  if (s === d - 1 && (i = false, u(), i)) throw new Error("The graph contains a negative weight cycle");
  return o;
}
function zn(e) {
  let n = {}, t = [], r;
  function o(i) {
    var s, a;
    i in n || (n[i] = true, r.push(i), (s = e.successors(i)) == null || s.forEach(o), (a = e.predecessors(i)) == null || a.forEach(o));
  }
  return e.nodes().forEach(function(i) {
    r = [], o(i), r.length && t.push(r);
  }), t;
}
var Pe = class {
  constructor() {
    this._arr = [], this._keyIndices = {};
  }
  size() {
    return this._arr.length;
  }
  keys() {
    return this._arr.map((e) => e.key);
  }
  has(e) {
    return e in this._keyIndices;
  }
  priority(e) {
    let n = this._keyIndices[e];
    if (n !== void 0) return this._arr[n].priority;
  }
  min() {
    if (this.size() === 0) throw new Error("Queue underflow");
    return this._arr[0].key;
  }
  add(e, n) {
    let t = this._keyIndices, r = String(e);
    if (!(r in t)) {
      let o = this._arr, i = o.length;
      return t[r] = i, o.push({ key: r, priority: n }), this._decrease(i), true;
    }
    return false;
  }
  removeMin() {
    if (this.size() === 0) throw new Error("Queue underflow");
    this._swap(0, this._arr.length - 1);
    let e = this._arr.pop();
    return delete this._keyIndices[e.key], this._heapify(0), e.key;
  }
  decrease(e, n) {
    let t = this._keyIndices[e];
    if (t === void 0) throw new Error(`Key not found: ${e}`);
    let r = this._arr[t].priority;
    if (n > r) throw new Error(`New priority is greater than current priority. Key: ${e} Old: ${r} New: ${n}`);
    this._arr[t].priority = n, this._decrease(t);
  }
  _heapify(e) {
    let n = this._arr, t = 2 * e, r = t + 1, o = e;
    t < n.length && (o = n[t].priority < n[o].priority ? t : o, r < n.length && (o = n[r].priority < n[o].priority ? r : o), o !== e && (this._swap(e, o), this._heapify(o)));
  }
  _decrease(e) {
    let n = this._arr, t = n[e].priority, r;
    for (; e !== 0 && (r = e >> 1, !(n[r].priority < t)); ) this._swap(e, r), e = r;
  }
  _swap(e, n) {
    let t = this._arr, r = this._keyIndices, o = t[e], i = t[n];
    t[e] = i, t[n] = o, r[i.key] = e, r[o.key] = n;
  }
};
var Hn = () => 1;
function J(e, n, t, r) {
  let o = function(i) {
    var s;
    return (s = e.outEdges(i)) != null ? s : [];
  };
  return qn(e, String(n), t || Hn, r || o);
}
function qn(e, n, t, r) {
  let o = {}, i = new Pe(), s, a, l = function(u) {
    let d = u.v !== s ? u.v : u.w, c = o[d];
    if (!c) return;
    let f = t(u), h2 = a.distance + f;
    if (f < 0) throw new Error("dijkstra does not allow negative edge weights. Bad edge: " + u + " Weight: " + f);
    h2 < c.distance && (c.distance = h2, c.predecessor = s, i.decrease(d, h2));
  };
  for (e.nodes().forEach(function(u) {
    let d = u === n ? 0 : Number.POSITIVE_INFINITY;
    o[u] = { distance: d, predecessor: "" }, i.add(u, d);
  }); i.size() > 0; ) {
    s = i.removeMin();
    let u = o[s];
    if (!u || u.distance === Number.POSITIVE_INFINITY) break;
    a = u, r(s).forEach(l);
  }
  return o;
}
function $n(e, n, t) {
  return e.nodes().reduce(function(r, o) {
    return r[o] = J(e, o, n, t), r;
  }, {});
}
function Fe(e) {
  let n = 0, t = [], r = {}, o = [];
  function i(s) {
    var a;
    let l = r[s] = { onStack: true, lowlink: n, index: n++ };
    if (t.push(s), (a = e.successors(s)) == null || a.forEach(function(u) {
      if (u in r) {
        let d = r[u];
        d != null && d.onStack && (l.lowlink = Math.min(l.lowlink, d.index));
      } else {
        i(u);
        let d = r[u];
        d && (l.lowlink = Math.min(l.lowlink, d.lowlink));
      }
    }), l.lowlink === l.index) {
      let u = [], d;
      do {
        d = t.pop();
        let c = r[d];
        c && (c.onStack = false), u.push(d);
      } while (s !== d);
      o.push(u);
    }
  }
  return e.nodes().forEach(function(s) {
    s in r || i(s);
  }), o;
}
function Un(e) {
  return Fe(e).filter(function(n) {
    var t;
    let r = n[0];
    return r ? n.length > 1 || n.length === 1 && ((t = e.outEdges(r, r)) != null ? t : []).length > 0 : false;
  });
}
var Jn = () => 1;
function Kn(e, n, t) {
  return Qn(e, n || Jn, t || function(r) {
    var o;
    return (o = e.outEdges(r)) != null ? o : [];
  });
}
function Qn(e, n, t) {
  let r = {}, o = e.nodes();
  return o.forEach(function(i) {
    let s = {};
    r[i] = s, s[i] = { distance: 0, predecessor: "" }, o.forEach(function(a) {
      i !== a && (s[a] = { distance: Number.POSITIVE_INFINITY, predecessor: "" });
    }), t(i).forEach(function(a) {
      let l = a.v === i ? a.w : a.v, u = n(a);
      s[l] = { distance: u, predecessor: i };
    });
  }), o.forEach(function(i) {
    let s = r[i];
    s && o.forEach(function(a) {
      let l = r[a];
      l && o.forEach(function(u) {
        let d = l[i], c = s[u], f = l[u];
        if (d && c && f) {
          let h2 = d.distance + c.distance;
          h2 < f.distance && (f.distance = h2, f.predecessor = c.predecessor);
        }
      });
    });
  }), r;
}
var K = class extends Error {
  constructor(e) {
    super(e), this.name = "CycleException";
  }
};
function De(e) {
  let n = {}, t = {}, r = [];
  function o(i) {
    var s;
    if (i in t) throw new K();
    i in n || (t[i] = true, n[i] = true, (s = e.predecessors(i)) == null || s.forEach(o), delete t[i], r.push(i));
  }
  if (e.sinks().forEach(o), Object.keys(n).length !== e.nodeCount()) throw new K();
  return r;
}
function Zn(e) {
  try {
    De(e);
  } catch (n) {
    if (n instanceof K) return false;
    throw n;
  }
  return true;
}
function et(e, n, t, r, o) {
  Array.isArray(n) || (n = [n]);
  let i = (a) => {
    var l;
    return (l = e.isDirected() ? e.successors(a) : e.neighbors(a)) != null ? l : [];
  }, s = {};
  return n.forEach(function(a) {
    if (!e.hasNode(a)) throw new Error("Graph does not have node: " + a);
    o = Ae(e, a, t === "post", s, i, r, o);
  }), o;
}
function Ae(e, n, t, r, o, i, s) {
  return n in r || (r[n] = true, t || (s = i(s, n)), o(n).forEach(function(a) {
    s = Ae(e, a, t, r, o, i, s);
  }), t && (s = i(s, n))), s;
}
function Ve(e, n, t) {
  return et(e, n, t, function(r, o) {
    return r.push(o), r;
  }, []);
}
function nt(e, n) {
  return Ve(e, n, "post");
}
function tt(e, n) {
  return Ve(e, n, "pre");
}
function rt(e, n) {
  var t;
  let r = new Q(), o = {}, i = new Pe(), s;
  function a(d) {
    let c = d.v === s ? d.w : d.v, f = i.priority(c);
    if (f !== void 0) {
      let h2 = n(d);
      h2 < f && (o[c] = s, i.decrease(c, h2));
    }
  }
  if (e.nodeCount() === 0) return r;
  e.nodes().forEach(function(d) {
    i.add(d, Number.POSITIVE_INFINITY), r.setNode(d);
  });
  let l = e.nodes()[0];
  l !== void 0 && i.decrease(l, 0);
  let u = false;
  for (; i.size() > 0; ) {
    if (s = i.removeMin(), s in o) r.setEdge(s, o[s]);
    else {
      if (u) throw new Error("Input graph is not connected: " + e);
      u = true;
    }
    (t = e.nodeEdges(s)) == null || t.forEach(a);
  }
  return r;
}
function ot(e, n, t, r) {
  return it(e, n, t, r != null ? r : (o) => {
    var i;
    return (i = e.outEdges(o)) != null ? i : [];
  });
}
function it(e, n, t, r) {
  if (t === void 0) return J(e, n, t, r);
  let o = false, i = e.nodes();
  for (let s = 0; s < i.length; s++) {
    let a = i[s];
    if (a === void 0) continue;
    let l = r(a);
    for (let u = 0; u < l.length; u++) {
      let d = l[u];
      if (!d) continue;
      let c = d.v === a ? d.v : d.w, f = c === d.v ? d.w : d.v;
      t({ v: c, w: f }) < 0 && (o = true);
    }
    if (o) return Me(e, n, t, r);
  }
  return J(e, n, t, r);
}
var T = Q;
function M(e, n, t, r) {
  let o = r;
  for (; e.hasNode(o); ) o = $2(r);
  return t.dummy = n, e.setNode(o, t), o;
}
function Ye(e) {
  let n = new T().setGraph(e.graph());
  return e.nodes().forEach((t) => n.setNode(t, e.node(t))), e.edges().forEach((t) => {
    let r = n.edge(t.v, t.w) || { weight: 0, minlen: 1 }, o = e.edge(t);
    n.setEdge(t.v, t.w, { weight: r.weight + o.weight, minlen: Math.max(r.minlen, o.minlen) });
  }), n;
}
function Z(e) {
  let n = new T({ multigraph: e.isMultigraph() }).setGraph(e.graph());
  return e.nodes().forEach((t) => {
    e.children(t).length || n.setNode(t, e.node(t));
  }), e.edges().forEach((t) => {
    n.setEdge(t, e.edge(t));
  }), n;
}
function se(e, n) {
  let t = e.x, r = e.y, o = n.x - t, i = n.y - r, s = e.width / 2, a = e.height / 2;
  if (!o && !i) throw new Error("Not possible to find intersection inside of the rectangle");
  let l, u;
  return o === 0 || Math.abs(i) * s > Math.abs(o) * a ? (i < 0 && (a = -a), l = a * o / i, u = a) : (o < 0 && (s = -s), l = s, u = s * i / o), { x: t + l, y: r + u };
}
function P(e) {
  let n = D(de(e) + 1).map(() => []);
  return e.nodes().forEach((t) => {
    let r = e.node(t), o = r.rank;
    o !== void 0 && (n[o] || (n[o] = []), n[o][r.order] = t);
  }), n;
}
function Be(e) {
  let n = e.nodes().map((r) => {
    let o = e.node(r).rank;
    return o === void 0 ? Number.MAX_VALUE : o;
  }), t = S(Math.min, n);
  e.nodes().forEach((r) => {
    let o = e.node(r);
    Object.hasOwn(o, "rank") && (o.rank -= t);
  });
}
function We(e) {
  let n = e.nodes().map((s) => e.node(s).rank).filter((s) => s !== void 0), t = S(Math.min, n), r = [];
  e.nodes().forEach((s) => {
    let a = e.node(s).rank - t;
    r[a] || (r[a] = []), r[a].push(s);
  });
  let o = 0, i = e.graph().nodeRankFactor;
  Array.from(r).forEach((s, a) => {
    s === void 0 && a % i !== 0 ? --o : s !== void 0 && o && s.forEach((l) => e.node(l).rank += o);
  });
}
function ae(e, n, t, r) {
  let o = { width: 0, height: 0 };
  return arguments.length >= 4 && (o.rank = t, o.order = r), M(e, "border", o, n);
}
function st(e, n = Xe) {
  let t = [];
  for (let r = 0; r < e.length; r += n) {
    let o = e.slice(r, r + n);
    t.push(o);
  }
  return t;
}
var Xe = 65535;
function S(e, n) {
  if (n.length > Xe) {
    let t = st(n);
    return e(...t.map((r) => e(...r)));
  } else return e(...n);
}
function de(e) {
  let t = e.nodes().map((r) => {
    let o = e.node(r).rank;
    return o === void 0 ? Number.MIN_VALUE : o;
  });
  return S(Math.max, t);
}
function ze(e, n) {
  let t = { lhs: [], rhs: [] };
  return e.forEach((r) => {
    n(r) ? t.lhs.push(r) : t.rhs.push(r);
  }), t;
}
function le(e, n) {
  let t = Date.now();
  try {
    return n();
  } finally {
    console.log(e + " time: " + (Date.now() - t) + "ms");
  }
}
function q(e, n) {
  return n();
}
var at = 0;
function $2(e) {
  let n = ++at;
  return e + ("" + n);
}
function D(e, n, t = 1) {
  n == null && (n = e, e = 0);
  let r = (i) => i < n;
  t < 0 && (r = (i) => n < i);
  let o = [];
  for (let i = e; r(i); i += t) o.push(i);
  return o;
}
function W(e, n) {
  let t = {};
  for (let r of n) e[r] !== void 0 && (t[r] = e[r]);
  return t;
}
function X(e, n) {
  let t;
  return typeof n == "string" ? t = (r) => r[n] : t = n, Object.entries(e).reduce((r, [o, i]) => (r[o] = t(i, o), r), {});
}
function He(e, n) {
  return e.reduce((t, r, o) => (t[r] = n[o], t), {});
}
var V = "\0";
function ee(e, n, t) {
  var u, d, c, f, h2, g;
  if (!(e && n && t && n.dummy === "edge" && t.dummy === "edge" && n.edgeObj && t.edgeObj && e[n.edgeObj.v] && e[t.edgeObj.v] && e[n.edgeObj.w] && e[t.edgeObj.w])) return 0;
  let r = true;
  n.edgeObj.w === t.edgeObj.w && (r = false);
  let o = r ? (d = (u = e[n.edgeObj.v]) == null ? void 0 : u.rank) != null ? d : NaN + 1 : (f = (c = e[n.edgeObj.w]) == null ? void 0 : c.rank) != null ? f : NaN - 1, i = Object.entries(e).find((m) => {
    var L, y;
    return ((L = m[1].edgeObj) == null ? void 0 : L.v) === n.edgeObj.v && ((y = m[1].edgeObj) == null ? void 0 : y.w) === n.edgeObj.w && m[1].rank === o;
  }), s = Object.entries(e).find((m) => {
    var L, y;
    return ((L = m[1].edgeObj) == null ? void 0 : L.v) === t.edgeObj.v && ((y = m[1].edgeObj) == null ? void 0 : y.w) === t.edgeObj.w && m[1].rank === o;
  });
  if (!i || !s) return 0;
  let a = (h2 = i[1].order) != null ? h2 : NaN, l = (g = s[1].order) != null ? g : NaN;
  return isNaN(a - l) ? 0 : a - l;
}
var ue = "3.1.3";
var ce = class {
  constructor() {
    je(this, "_sentinel");
    let n = {};
    n._next = n._prev = n, this._sentinel = n;
  }
  dequeue() {
    let n = this._sentinel, t = n._prev;
    if (t !== n) return qe(t), t;
  }
  enqueue(n) {
    let t = this._sentinel;
    n._prev && n._next && qe(n), n._next = t._next, t._next._prev = n, t._next = n, n._prev = t;
  }
  toString() {
    let n = [], t = this._sentinel, r = t._prev;
    for (; r !== t; ) n.push(JSON.stringify(r, dt)), r = r._prev;
    return "[" + n.join(", ") + "]";
  }
};
function qe(e) {
  e._prev._next = e._next, e._next._prev = e._prev, delete e._next, delete e._prev;
}
function dt(e, n) {
  if (e !== "_next" && e !== "_prev") return n;
}
var $e = ce;
var lt = () => 1;
function be(e, n) {
  if (e.nodeCount() <= 1) return [];
  let t = ct(e, n || lt);
  return ut(t.graph, t.buckets, t.zeroIdx).flatMap((o) => e.outEdges(o.v, o.w) || []);
}
function ut(e, n, t) {
  var a;
  let r = [], o = n[n.length - 1], i = n[0], s;
  for (; e.nodeCount(); ) {
    for (; s = i.dequeue(); ) fe(e, n, t, s);
    for (; s = o.dequeue(); ) fe(e, n, t, s);
    if (e.nodeCount()) {
      for (let l = n.length - 2; l > 0; --l) if (s = (a = n[l]) == null ? void 0 : a.dequeue(), s) {
        r = r.concat(fe(e, n, t, s, true) || []);
        break;
      }
    }
  }
  return r;
}
function fe(e, n, t, r, o) {
  let i = [], s = o ? i : void 0;
  return (e.inEdges(r.v) || []).forEach((a) => {
    let l = e.edge(a), u = e.node(a.v);
    o && i.push({ v: a.v, w: a.w }), u.out -= l, he(n, t, u);
  }), (e.outEdges(r.v) || []).forEach((a) => {
    let l = e.edge(a), u = a.w, d = e.node(u);
    d.in -= l, he(n, t, d);
  }), e.removeNode(r.v), s;
}
function ct(e, n) {
  let t = new T(), r = 0, o = 0;
  e.nodes().forEach((a) => {
    t.setNode(a, { v: a, in: 0, out: 0 });
  }), e.edges().forEach((a) => {
    let l = t.edge(a.v, a.w) || 0, u = n(a), d = l + u;
    t.setEdge(a.v, a.w, d);
    let c = t.node(a.v), f = t.node(a.w);
    o = Math.max(o, c.out += u), r = Math.max(r, f.in += u);
  });
  let i = ft(o + r + 3).map(() => new $e()), s = r + 1;
  return t.nodes().forEach((a) => {
    he(i, s, t.node(a));
  }), { graph: t, buckets: i, zeroIdx: s };
}
function he(e, n, t) {
  var r, o, i;
  t.out ? t.in ? (i = e[t.out - t.in + n]) == null || i.enqueue(t) : (o = e[e.length - 1]) == null || o.enqueue(t) : (r = e[0]) == null || r.enqueue(t);
}
function ft(e) {
  let n = [];
  for (let t = 0; t < e; t++) n.push(t);
  return n;
}
function Ue(e, n) {
  (e.graph().acyclicer === "greedy" ? be(e, r(e)) : ht(e, n != null ? n : null)).forEach((o) => {
    let i = e.edge(o);
    e.removeEdge(o), i.forwardName = o.name, i.reversed = true, e.setEdge(o.w, o.v, i, $2("rev"));
  });
  function r(o) {
    return (i) => o.edge(i).weight;
  }
}
function ht(e, n) {
  let t = [], r = {}, o = {};
  function i(l) {
    Object.hasOwn(o, l) || (o[l] = true, r[l] = true, e.outEdges(l).forEach((u) => {
      Object.hasOwn(r, u.w) ? t.push(u) : i(u.w);
    }), delete r[l]);
  }
  function s(l) {
    var u;
    Object.hasOwn(o, l) || (o[l] = true, r[l] = true, (u = e.outEdges(l)) == null || u.forEach((d) => {
      var c, f;
      Object.hasOwn(r, d.w) || ((c = n.node(l)) == null ? void 0 : c.rank) > ((f = n.node(d.w)) == null ? void 0 : f.rank) && bt(e, d.w, d) ? t.push(d) : s(d.w);
    }), delete r[l]);
  }
  let a = i;
  return n && typeof n.node == "function" && (a = s), e.sources().forEach(a), e.nodes().forEach(a), t;
}
function Je(e) {
  e.edges().forEach((n) => {
    let t = e.edge(n);
    if (t.reversed) {
      e.removeEdge(n);
      let r = t.forwardName;
      delete t.reversed, delete t.forwardName, e.setEdge(n.w, n.v, t, r);
    }
  });
}
function bt(e, n, t) {
  let r = /* @__PURE__ */ new Set();
  function o(i) {
    var s;
    if (e.sources().includes(i)) return true;
    r.add(i);
    for (let a of (s = e.inEdges(i)) != null ? s : []) if (!(a.v === t.v && a.w === t.w) && !r.has(a.v) && o(a.v)) return true;
    return false;
  }
  return o(n);
}
function Ke(e) {
  e.graph().dummyChains = [], e.edges().forEach((n) => pt(e, n));
}
function pt(e, n) {
  let t = n.v, r = e.node(t).rank, o = n.w, i = e.node(o).rank, s = n.name, a = e.edge(n), l = a.labelRank;
  if (i === r + 1) return;
  e.removeEdge(n);
  let u, d, c;
  for (c = 0, ++r; r < i; ++c, ++r) a.points = [], d = { width: 0, height: 0, edgeLabel: a, edgeObj: n, rank: r }, u = M(e, "edge", d, "_d"), r === l && (d.width = a.width, d.height = a.height, d.dummy = "edge-label", d.labelpos = a.labelpos), e.setEdge(t, u, { weight: a.weight }, s), c === 0 && e.graph().dummyChains.push(u), t = u;
  e.setEdge(t, o, { weight: a.weight }, s);
}
function Qe(e) {
  e.graph().dummyChains.forEach((n) => {
    let t = e.node(n), r = t.edgeLabel, o;
    for (e.setEdge(t.edgeObj, r); t.dummy; ) o = e.successors(n)[0], e.removeNode(n), r.points.push({ x: t.x, y: t.y }), t.dummy === "edge-label" && (r.x = t.x, r.y = t.y, r.width = t.width, r.height = t.height), n = o, t = e.node(n);
  });
}
function U(e) {
  let n = {};
  function t(r) {
    let o = e.node(r);
    if (Object.hasOwn(n, r)) return o.rank;
    n[r] = true;
    let i = e.outEdges(r), s = i ? i.map((l) => l == null ? Number.POSITIVE_INFINITY : t(l.w) - e.edge(l).minlen) : [], a = S(Math.min, s);
    return a === Number.POSITIVE_INFINITY && (a = 0), o.rank = a;
  }
  e.sources().forEach(t);
}
function A(e, n) {
  return e.node(n.w).rank - e.node(n.v).rank - e.edge(n).minlen;
}
var ne = Et;
function Et(e) {
  let n = new T({ directed: false }), t = e.nodes();
  if (t.length === 0) throw new Error("Graph must have at least one node");
  let r = t[0], o = e.nodeCount();
  n.setNode(r, {});
  let i, s;
  for (; Lt(n, e) < o && (i = yt(n, e), !!i); ) s = n.hasNode(i.v) ? A(e, i) : -A(e, i), wt(n, e, s);
  return n;
}
function Lt(e, n) {
  function t(r) {
    let o = n.nodeEdges(r);
    o && o.forEach((i) => {
      let s = i.v, a = r === s ? i.w : s;
      !e.hasNode(a) && !A(n, i) && (e.setNode(a, {}), e.setEdge(r, a, {}), t(a));
    });
  }
  return e.nodes().forEach(t), e.nodeCount();
}
function yt(e, n) {
  return n.edges().reduce((r, o) => {
    let i = Number.POSITIVE_INFINITY;
    return e.hasNode(o.v) !== e.hasNode(o.w) && (i = A(n, o)), i < r[0] ? [i, o] : r;
  }, [Number.POSITIVE_INFINITY, null])[1];
}
function wt(e, n, t) {
  e.nodes().forEach((r) => n.node(r).rank += t);
}
var { preorder: Nt, postorder: Gt } = H;
var en = Y;
Y.initLowLimValues = pe;
Y.initCutValues = ge;
Y.calcCutValue = nn;
Y.leaveEdge = rn;
Y.enterEdge = on;
Y.exchangeEdges = sn;
function Y(e) {
  e = Ye(e), U(e);
  let n = ne(e);
  pe(n), ge(n, e);
  let t, r;
  for (; t = rn(n); ) r = on(n, e, t), sn(n, e, t, r);
}
function ge(e, n) {
  let t = Gt(e, e.nodes());
  t = t.slice(0, t.length - 1), t.forEach((r) => vt(e, n, r));
}
function vt(e, n, t) {
  let o = e.node(t).parent, i = e.edge(t, o);
  i.cutvalue = nn(e, n, t);
}
function nn(e, n, t) {
  let o = e.node(t).parent, i = true, s = n.edge(t, o), a = 0;
  s || (i = false, s = n.edge(o, t)), a = s.weight;
  let l = n.nodeEdges(t);
  return l && l.forEach((u) => {
    let d = u.v === t, c = d ? u.w : u.v;
    if (c !== o) {
      let f = d === i, h2 = n.edge(u).weight;
      if (a += f ? h2 : -h2, xt(e, t, c)) {
        let m = e.edge(t, c).cutvalue;
        a += f ? -m : m;
      }
    }
  }), a;
}
function pe(e, n) {
  arguments.length < 2 && (n = e.nodes()[0]), tn(e, {}, 1, n);
}
function tn(e, n, t, r, o) {
  let i = t, s = e.node(r);
  n[r] = true;
  let a = e.neighbors(r);
  return a && a.forEach((l) => {
    Object.hasOwn(n, l) || (t = tn(e, n, t, l, r));
  }), s.low = i, s.lim = t++, o ? s.parent = o : delete s.parent, t;
}
function rn(e) {
  return e.edges().find((n) => e.edge(n).cutvalue < 0);
}
function on(e, n, t) {
  let r = t.v, o = t.w;
  n.hasEdge(r, o) || (r = t.w, o = t.v);
  let i = e.node(r), s = e.node(o), a = i, l = false;
  return i.lim > s.lim && (a = s, l = true), n.edges().filter((d) => l === Ze(e, e.node(d.v), a) && l !== Ze(e, e.node(d.w), a)).reduce((d, c) => A(n, c) < A(n, d) ? c : d);
}
function sn(e, n, t, r) {
  let o = t.v, i = t.w;
  e.removeEdge(o, i), e.setEdge(r.v, r.w, {}), pe(e), ge(e, n), kt(e, n);
}
function kt(e, n) {
  let t = e.nodes().find((o) => !e.node(o).parent);
  if (!t) return;
  let r = Nt(e, [t]);
  r = r.slice(1), r.forEach((o) => {
    let s = e.node(o).parent, a = n.edge(o, s), l = false;
    a || (a = n.edge(s, o), l = true), n.node(o).rank = n.node(s).rank + (l ? a.minlen : -a.minlen);
  });
}
function xt(e, n, t) {
  return e.hasEdge(n, t);
}
function Ze(e, n, t) {
  return t.low <= n.lim && n.lim <= t.lim;
}
var dn = _t;
function _t(e) {
  let n = e.graph().ranker;
  if (typeof n == "function") return n(e);
  switch (n) {
    case "network-simplex":
      an(e);
      break;
    case "tight-tree":
      Ct(e);
      break;
    case "longest-path":
      Ot(e);
      break;
    case "none":
      break;
    default:
      an(e);
  }
}
var Ot = U;
function Ct(e) {
  U(e), ne(e);
}
function an(e) {
  en(e);
}
var ln = Tt;
function Tt(e) {
  let n = Rt(e), t = e.graph();
  if (!Array.isArray(t.dummyChains)) return;
  t.dummyChains.forEach((o) => {
    let i = e.node(o), s = i.edgeObj, a = jt(e, n, s.v, s.w), l = a.path, u = a.lca, d = 0, c = l[d], f = true;
    for (; o !== s.w; ) {
      if (i = e.node(o), f) {
        for (; (c = l[d]) !== u && e.node(c).maxRank < i.rank; ) d++;
        c === u && (f = false);
      }
      if (!f) {
        for (; d < l.length - 1 && e.node(l[d + 1]).minRank <= i.rank; ) d++;
        c = l[d];
      }
      c !== void 0 && e.setParent(o, c), o = e.successors(o)[0];
    }
  });
}
function jt(e, n, t, r) {
  let o = [], i = [], s = Math.min(n[t].low, n[r].low), a = Math.max(n[t].lim, n[r].lim), l;
  l = t;
  do
    l = e.parent(l), o.push(l);
  while (l && (n[l].low > s || a > n[l].lim));
  let u = l, d = r;
  for (; (d = e.parent(d)) !== u; ) i.push(d);
  return { path: o.concat(i.reverse()), lca: u };
}
function Rt(e) {
  let n = {}, t = 0;
  function r(o) {
    let i = t;
    e.children(o).forEach(r), n[o] = { low: i, lim: t++ };
  }
  return e.children(V).forEach(r), n;
}
function un(e) {
  let n = M(e, "root", {}, "_root"), t = It(e), r = Object.values(t), o = S(Math.max, r) - 1, i = 2 * o + 1;
  e.graph().nestingRoot = n, e.edges().forEach((a) => e.edge(a).minlen *= i);
  let s = St(e) + 1;
  e.children(V).forEach((a) => {
    cn(e, n, i, s, o, t, a);
  }), e.graph().nodeRankFactor = i;
}
function cn(e, n, t, r, o, i, s) {
  var c;
  let a = e.children(s);
  if (!a.length) {
    s !== n && e.setEdge(n, s, { weight: 0, minlen: t });
    return;
  }
  let l = ae(e, "_bt"), u = ae(e, "_bb"), d = e.node(s);
  e.setParent(l, s), d.borderTop = l, e.setParent(u, s), d.borderBottom = u, a.forEach((f) => {
    var b;
    cn(e, n, t, r, o, i, f);
    let h2 = e.node(f), g = h2.borderTop ? h2.borderTop : f, m = h2.borderBottom ? h2.borderBottom : f, L = h2.borderTop ? r : 2 * r, y = g !== m ? 1 : o - ((b = i[s]) != null ? b : 0) + 1;
    e.setEdge(l, g, { weight: L, minlen: y, nestingEdge: true }), e.setEdge(m, u, { weight: L, minlen: y, nestingEdge: true });
  }), e.parent(s) || e.setEdge(n, l, { weight: 0, minlen: o + ((c = i[s]) != null ? c : 0) });
}
function It(e) {
  let n = {};
  function t(r, o) {
    let i = e.children(r);
    i && i.length && i.forEach((s) => t(s, o + 1)), n[r] = o;
  }
  return e.children(V).forEach((r) => t(r, 1)), n;
}
function St(e) {
  return e.edges().reduce((n, t) => n + e.edge(t).weight, 0);
}
function fn(e) {
  let n = e.graph();
  e.removeNode(n.nestingRoot), delete n.nestingRoot, e.edges().forEach((t) => {
    e.edge(t).nestingEdge && e.removeEdge(t);
  });
}
var bn = Pt;
function Pt(e) {
  function n(t) {
    let r = e.children(t), o = e.node(t);
    if (r.length && r.forEach(n), o && Object.hasOwn(o, "minRank")) {
      o.borderLeft = [], o.borderRight = [];
      for (let i = o.minRank, s = o.maxRank + 1; i < s; ++i) hn(e, "borderLeft", "_bl", t, o, i), hn(e, "borderRight", "_br", t, o, i);
    }
  }
  e.children(V).forEach(n);
}
function hn(e, n, t, r, o, i) {
  let s = { width: 0, height: 0, rank: i, borderType: n }, a = o[n][i - 1], l = M(e, "border", s, t);
  o[n][i] = l, e.setParent(l, r), a && e.setEdge(a, l, { weight: 1 });
}
function pn(e) {
  var t;
  let n = (t = e.graph().rankdir) == null ? void 0 : t.toLowerCase();
  (n === "lr" || n === "rl") && En(e);
}
function mn(e) {
  var t;
  let n = (t = e.graph().rankdir) == null ? void 0 : t.toLowerCase();
  (n === "bt" || n === "rl") && Ft(e), (n === "lr" || n === "rl") && (Dt(e), En(e));
}
function En(e) {
  e.nodes().forEach((n) => gn(e.node(n))), e.edges().forEach((n) => gn(e.edge(n)));
}
function gn(e) {
  let n = e.width;
  e.width = e.height, e.height = n;
}
function Ft(e) {
  e.nodes().forEach((n) => me(e.node(n))), e.edges().forEach((n) => {
    var r;
    let t = e.edge(n);
    (r = t.points) == null || r.forEach(me), Object.hasOwn(t, "y") && me(t);
  });
}
function me(e) {
  e.y = -e.y;
}
function Dt(e) {
  e.nodes().forEach((n) => Ee(e.node(n))), e.edges().forEach((n) => {
    var r;
    let t = e.edge(n);
    (r = t.points) == null || r.forEach(Ee), Object.hasOwn(t, "x") && Ee(t);
  });
}
function Ee(e) {
  let n = e.x;
  e.x = e.y, e.y = n;
}
function Le(e, n = null) {
  let t = {}, r = e.nodes().filter((d) => !e.children(d).length), o = r.map((d) => e.node(d).rank), i = S(Math.max, o), s = D(i + 1).map(() => []);
  function a(d) {
    if (t[d]) return;
    t[d] = true;
    let c = e.node(d);
    s[c.rank].push(d);
    let f = e.successors(d);
    f && [...f].sort((g, m) => u(g, m)).forEach(a);
  }
  r.sort((d, c) => e.node(d).rank - e.node(c).rank).forEach(a);
  function u(d, c) {
    let f = e.node(d), h2 = e.node(c);
    return ee(n, f, h2);
  }
  return s;
}
function ye(e, n) {
  let t = 0;
  for (let r = 1; r < n.length; ++r) t += Vt(e, n[r - 1], n[r]);
  return t;
}
function Vt(e, n, t) {
  let r = He(t, t.map((u, d) => d)), o = n.flatMap((u) => {
    let d = e.outEdges(u);
    return d ? d.map((c) => ({ pos: r[c.w], weight: e.edge(c).weight })).sort((c, f) => c.pos - f.pos) : [];
  }), i = 1;
  for (; i < t.length; ) i <<= 1;
  let s = 2 * i - 1;
  i -= 1;
  let a = new Array(s).fill(0), l = 0;
  return o.forEach((u) => {
    let d = u.pos + i;
    a[d] += u.weight;
    let c = 0;
    for (; d > 0; ) d % 2 && (c += a[d + 1]), d = d - 1 >> 1, a[d] += u.weight;
    l += u.weight * c;
  }), l;
}
function we(e, n = []) {
  return n.map((t) => {
    let r = e.inEdges(t);
    if (!r || !r.length) return { v: t };
    {
      let o = r.reduce((i, s) => {
        let a = e.edge(s), l = e.node(s.v);
        return { sum: i.sum + a.weight * l.order, weight: i.weight + a.weight };
      }, { sum: 0, weight: 0 });
      return { v: t, barycenter: o.sum / o.weight, weight: o.weight };
    }
  });
}
function Ne(e, n) {
  let t = {};
  e.forEach((o, i) => {
    let s = { indegree: 0, in: [], out: [], vs: [o.v], i };
    o.barycenter !== void 0 && (s.barycenter = o.barycenter, s.weight = o.weight), t[o.v] = s;
  }), n.edges().forEach((o) => {
    let i = t[o.v], s = t[o.w];
    i !== void 0 && s !== void 0 && (s.indegree++, i.out.push(s));
  });
  let r = Object.values(t).filter((o) => !o.indegree);
  return Yt(r);
}
function Yt(e) {
  let n = [];
  function t(o) {
    return (i) => {
      i.merged || (i.barycenter === void 0 || o.barycenter === void 0 || i.barycenter >= o.barycenter) && Bt(o, i);
    };
  }
  function r(o) {
    return (i) => {
      i.in.push(o), --i.indegree === 0 && e.push(i);
    };
  }
  for (; e.length; ) {
    let o = e.pop();
    n.push(o), o.in.reverse().forEach(t(o)), o.out.forEach(r(o));
  }
  return n.filter((o) => !o.merged).map((o) => W(o, ["vs", "i", "barycenter", "weight"]));
}
function Bt(e, n) {
  let t = 0, r = 0;
  e.weight && (t += e.barycenter * e.weight, r += e.weight), n.weight && (t += n.barycenter * n.weight, r += n.weight), e.vs = n.vs.concat(e.vs), e.barycenter = t / r, e.weight = r, e.i = Math.min(n.i, e.i), n.merged = true;
}
function Ge(e, n, t, r, o) {
  let i = {}, s = null, a = null, l = o;
  typeof n == "boolean" ? (l = n, i = {}) : n && (i = n, s = t != null ? t : null, a = r != null ? r : null);
  let u = ze(e, (y) => Object.hasOwn(y, "barycenter")), d = u.lhs, c = u.rhs.sort((y, b) => b.i - y.i), f = [], h2 = 0, g = 0, m = 0;
  d.sort(Wt(a, s, !!l));
  for (let [y, b] of Object.entries(i)) {
    let p = d.findIndex((E) => E.vs[0] === y);
    d.splice(p + 1, 0, b);
  }
  m = Ln(f, c, m), d.forEach((y) => {
    m += y.vs.length, f.push(y.vs), h2 += y.barycenter * y.weight, g += y.weight, m = Ln(f, c, m);
  });
  let L = { vs: f.flat(1) };
  return g && (L.barycenter = h2 / g, L.weight = g), L;
}
function Ln(e, n, t) {
  let r;
  for (; n.length && (r = n[n.length - 1]).i <= t; ) n.pop(), e.push(r.vs), t++;
  return t;
}
function Wt(e, n, t) {
  return (r, o) => {
    if (r.barycenter < o.barycenter) return -1;
    if (r.barycenter > o.barycenter) return 1;
    if (e && (typeof r.vs[0] == "string" || typeof o.vs[0] == "string")) {
      let i = e.node(r.vs[0]), s = e.node(o.vs[0]), a = ee(n, i, s);
      if (a !== 0) return a;
    }
    return t ? o.i - r.i : r.i - o.i;
  };
}
function te(e, n, t, r, o) {
  var y, b, p, E, w, k, _, O, j, R, I;
  let i = null, s = o;
  typeof r == "boolean" ? (s = r, i = null) : r !== void 0 && (i = r);
  let a = e.children(n), l = e.node(n), u = l ? l.borderLeft : void 0, d = l ? l.borderRight : void 0, c = {};
  u && (a = a.filter((v) => v !== u && v !== d));
  let f = we(e, a);
  f.forEach((v) => {
    if (e.children(v.v).length) {
      let { result: x } = te(e, v.v, t, i, s);
      c[v.v] = x, Object.hasOwn(x, "barycenter") && zt(v, x);
    }
  });
  let h2 = Ne(f, t);
  Xt(h2, c);
  let g = {}, m = false;
  for (let v = 0; v < h2.length; v++) for (let x = v + 1; x < h2.length; x++) if (!(!h2[v] || !h2[x] || !((y = h2[v]) != null && y.barycenter) || !((b = h2[x]) != null && b.barycenter)) && ((p = h2[v]) == null ? void 0 : p.barycenter) === h2[x].barycenter) {
    let G = (w = (E = h2[v]) == null ? void 0 : E.vs[0]) != null ? w : "", N = (_ = (k = h2[x]) == null ? void 0 : k.vs[0]) != null ? _ : "", C = e.node(G), B = e.node(N);
    if (C.dummy === "edge" && B.dummy === "edge" && ((O = C.edgeObj) == null ? void 0 : O.v) === ((j = B.edgeObj) == null ? void 0 : j.v) && ((R = C.edgeObj) == null ? void 0 : R.w) === ((I = B.edgeObj) == null ? void 0 : I.w)) if (C.edgeLabel.reversed) {
      g[N] = h2[v], h2.splice(v, 1), v--;
      break;
    } else g[G] || (g[G] = h2[x], h2.splice(x, 1), x--);
    else m = true;
  }
  let L = Ge(h2, g, i, e, s);
  if (u && d) {
    L.vs = [u, L.vs, d].flat(1);
    let v = e.predecessors(u);
    if (v && v.length) {
      let x = e.node(v[0]), G = e.predecessors(d), N = e.node(G[0]);
      Object.hasOwn(L, "barycenter") || (L.barycenter = 0, L.weight = 0), L.barycenter = (L.barycenter * L.weight + x.order + N.order) / (L.weight + 2), L.weight += 2;
    }
  }
  return Object.defineProperty(L, "result", { value: L, enumerable: false, configurable: true, writable: true }), Object.defineProperty(L, "usedBias", { value: m, enumerable: false, configurable: true, writable: true }), L;
}
function Xt(e, n) {
  e.forEach((t) => {
    t.vs = t.vs.flatMap((r) => n[r] ? n[r].vs : r);
  });
}
function zt(e, n) {
  e.barycenter !== void 0 ? (e.barycenter = (e.barycenter * e.weight + n.barycenter * n.weight) / (e.weight + n.weight), e.weight += n.weight) : (e.barycenter = n.barycenter, e.weight = n.weight);
}
function ve(e, n, t, r) {
  r || (r = e.nodes());
  let o = Ht(e), i = new T({ compound: true }).setGraph({ root: o }).setDefaultNodeLabel((s) => e.node(s));
  return r.forEach((s) => {
    let a = e.node(s), l = e.parent(s);
    if (a.rank === n || a.minRank <= n && n <= a.maxRank) {
      i.setNode(s), i.setParent(s, l || o);
      let u = e[t](s);
      u && u.forEach((d) => {
        let c = d.v === s ? d.w : d.v, f = i.edge(c, s), h2 = f !== void 0 ? f.weight : 0;
        i.setEdge(c, s, { weight: e.edge(d).weight + h2 });
      }), Object.hasOwn(a, "minRank") && i.setNode(s, { borderLeft: a.borderLeft[n], borderRight: a.borderRight[n] });
    }
  }), i;
}
function Ht(e) {
  let n;
  for (; e.hasNode(n = $2("_root")); ) ;
  return n;
}
function ke(e, n, t) {
  let r = {}, o;
  t.forEach((i) => {
    let s = e.parent(i), a, l;
    for (; s; ) {
      if (a = e.parent(s), a ? (l = r[a], r[a] = s) : (l = o, o = s), l && l !== s) {
        n.setEdge(l, s);
        return;
      }
      s = a;
    }
  });
}
function re(e, n = {}, t = null) {
  if (typeof n.customOrder == "function") {
    n.customOrder(e, re);
    return;
  }
  let r = de(e), o = yn(e, D(1, r + 1), "inEdges"), i = yn(e, D(r - 1, -1, -1), "outEdges"), s = Le(e, t);
  if (wn(e, s), n.disableOptimalOrderHeuristic) return;
  let a = Number.POSITIVE_INFINITY, l, u = n.constraints || [];
  for (let d = 0, c = 0; c < 4; ++d, ++c) {
    qt(d % 2 ? o : i, d % 4 >= 2, u, t), s = P(e);
    let f = ye(e, s);
    f < a ? (c = 0, l = Object.assign({}, s), a = f) : f === a && (l = structuredClone(s));
  }
  wn(e, l);
}
function yn(e, n, t) {
  let r = /* @__PURE__ */ new Map(), o = (i, s) => {
    r.has(i) || r.set(i, []), r.get(i).push(s);
  };
  for (let i of e.nodes()) {
    let s = e.node(i);
    if (typeof s.rank == "number" && o(s.rank, i), typeof s.minRank == "number" && typeof s.maxRank == "number") for (let a = s.minRank; a <= s.maxRank; a++) a !== s.rank && o(a, i);
  }
  return n.map(function(i) {
    return ve(e, i, t, r.get(i) || []);
  });
}
function qt(e, n, t, r) {
  let o = true, i = new T();
  e.forEach(function(s) {
    t.forEach((d) => i.setEdge(d.left, d.right));
    let a = s.graph().root, { result: l, usedBias: u } = te(s, a, i, r, o);
    n && u && (o = !o), l.vs.forEach((d, c) => s.node(d).order = c), ke(s, i, l.vs);
  });
}
function wn(e, n) {
  Object.values(n).forEach((t) => t.forEach((r, o) => e.node(r).order = o));
}
function $t(e, n) {
  let t = {};
  function r(o, i) {
    let s = 0, a = 0, l = o.length, u = i[i.length - 1];
    return i.forEach((d, c) => {
      let f = Jt(e, d), h2 = f ? e.node(f).order : l;
      (f || d === u) && (i.slice(a, c + 1).forEach((g) => {
        let m = e.predecessors(g);
        m && m.forEach((L) => {
          let y = e.node(L), b = y.order;
          (b < s || h2 < b) && !(y.dummy && e.node(g).dummy) && Gn(t, L, g);
        });
      }), a = c + 1, s = h2);
    }), i;
  }
  return n.length && n.reduce(r), t;
}
function Ut(e, n) {
  let t = {};
  function r(a) {
    var l;
    return ((l = e.node(a)) == null ? void 0 : l.dummy) === "border";
  }
  if (!e.nodes().some(r)) return t;
  function i(a, l, u, d, c) {
    D(l, u).forEach((f) => {
      let h2 = a[f];
      if (h2 !== void 0 && e.node(h2).dummy) {
        let g = e.predecessors(h2);
        g && g.forEach((m) => {
          if (m === void 0) return;
          let L = e.node(m);
          L.dummy && (L.order < d || L.order > c) && Gn(t, m, h2);
        });
      }
    });
  }
  function s(a, l) {
    let u = -1, d = -1, c = 0;
    return l.forEach((f, h2) => {
      if (r(f)) {
        let g = e.predecessors(f);
        if (g && g.length) {
          let m = g[0];
          if (m === void 0) return;
          d = e.node(m).order, i(l, c, h2, u, d), c = h2, u = d;
        }
      }
      i(l, c, l.length, d, a.length);
    }), l;
  }
  return n.length && n.reduce(s), t;
}
function Jt(e, n) {
  if (e.node(n).dummy) {
    let t = e.predecessors(n);
    if (t) return t.find((r) => e.node(r).dummy);
  }
}
function Gn(e, n, t) {
  if (n > t) {
    let o = n;
    n = t, t = o;
  }
  let r = e[n];
  r || (e[n] = r = {}), r[t] = true;
}
function Kt(e, n, t) {
  if (n > t) {
    let o = n;
    n = t, t = o;
  }
  let r = e[n];
  return r !== void 0 && Object.hasOwn(r, t);
}
function Qt(e, n, t, r, o) {
  let i = {}, s = {}, a = {};
  return n.forEach((l) => {
    l.forEach((u, d) => {
      i[u] = u, s[u] = u, a[u] = d;
    });
  }), n.forEach((l) => {
    let u = -1, d = -1, c = false, f = l, h2 = l.findIndex((g) => (o == null ? void 0 : o.includes(g)) || Nn(g, e, o));
    h2 > 0 && (f = [l[h2], ...l.slice(0, h2), ...l.slice(h2 + 1)], c = true), f.forEach((g) => {
      var L;
      let m = r(g);
      if (m && m.length) {
        o != null && o.includes(g) && (m = m.filter((p) => Nn(p, e, o)));
        let y = m.sort((p, E) => {
          let w = a[p], k = a[E];
          return (w !== void 0 ? w : 0) - (k !== void 0 ? k : 0);
        }), b = (y.length - 1) / 2;
        for (let p = Math.floor(b), E = Math.ceil(b); p <= E; ++p) {
          let w = y[p];
          if (w === void 0) continue;
          let k = a[w];
          if (k !== void 0 && s[g] === g && u < k && a[w] !== d && !Kt(t, g, w)) {
            let _ = i[w];
            _ !== void 0 && (s[w] = g, s[g] = i[g] = _, u = k, c && (u = -1, d = (L = a[w]) != null ? L : -1, c = false));
          }
        }
      }
    });
  }), { root: i, align: s };
}
function Zt(e, n, t, r, o = false) {
  let i = {}, s = er(e, n, t, o), a = o ? "borderLeft" : "borderRight";
  function l(h2, g) {
    let m = s.nodes().slice(), L = {}, y = m.pop();
    for (; y; ) {
      if (L[y]) h2(y);
      else {
        L[y] = true, m.push(y);
        for (let b of g(y)) m.push(b);
      }
      y = m.pop();
    }
  }
  function u(h2) {
    let g = s.inEdges(h2);
    g ? i[h2] = g.reduce((m, L) => {
      var p;
      let y = (p = i[L.v]) != null ? p : 0, b = s.edge(L);
      return Math.max(m, y + (b !== void 0 ? b : 0));
    }, 0) : i[h2] = 0;
  }
  function d(h2) {
    let g = s.outEdges(h2), m = Number.POSITIVE_INFINITY;
    g && (m = g.reduce((y, b) => {
      let p = i[b.w], E = s.edge(b);
      return Math.min(y, (p !== void 0 ? p : 0) - (E !== void 0 ? E : 0));
    }, Number.POSITIVE_INFINITY));
    let L = e.node(h2);
    m !== Number.POSITIVE_INFINITY && L.borderType !== a && (i[h2] = Math.max(i[h2] !== void 0 ? i[h2] : 0, m));
  }
  function c(h2) {
    return s.predecessors(h2) || [];
  }
  function f(h2) {
    return s.successors(h2) || [];
  }
  return l(u, c), l(d, f), Object.keys(r).forEach((h2) => {
    var m;
    let g = t[h2];
    g !== void 0 && (i[h2] = (m = i[g]) != null ? m : 0);
  }), i;
}
function er(e, n, t, r) {
  let o = new T(), i = e.graph(), s = or(i.nodesep, i.edgesep, r);
  return n.forEach((a) => {
    let l;
    a.forEach((u) => {
      let d = t[u];
      if (d !== void 0) {
        if (o.setNode(d), l !== void 0) {
          let c = t[l];
          if (c !== void 0) {
            let f = o.edge(c, d);
            o.setEdge(c, d, Math.max(s(e, u, l), f || 0));
          }
        }
        l = u;
      }
    });
  }), o;
}
function nr(e, n) {
  return Object.values(n).reduce((t, r) => {
    let o = Number.NEGATIVE_INFINITY, i = Number.POSITIVE_INFINITY;
    Object.entries(r).forEach(([a, l]) => {
      let u = ir(e, a) / 2;
      o = Math.max(l + u, o), i = Math.min(l - u, i);
    });
    let s = o - i;
    return s < t[0] && (t = [s, r]), t;
  }, [Number.POSITIVE_INFINITY, null])[1];
}
function tr(e, n) {
  let t = Object.values(n), r = S(Math.min, t), o = S(Math.max, t);
  ["u", "d"].forEach((i) => {
    ["l", "r"].forEach((s) => {
      let a = i + s, l = e[a];
      if (!l || l === n) return;
      let u = Object.values(l), d = r - S(Math.min, u);
      s !== "l" && (d = o - S(Math.max, u)), d && (e[a] = X(l, (c) => c + d));
    });
  });
}
function rr(e, n = void 0) {
  let t = e.ul;
  return t ? X(t, (r, o) => {
    var s, a;
    if (n) {
      let l = n.toLowerCase(), u = e[l];
      if (u && u[o] !== void 0) return u[o];
    }
    let i = Object.values(e).map((l) => {
      let u = l[o];
      return u !== void 0 ? u : 0;
    }).sort((l, u) => l - u);
    return (((s = i[1]) != null ? s : 0) + ((a = i[2]) != null ? a : 0)) / 2;
  }) : {};
}
function vn(e, n) {
  let t = P(e), r = Object.assign($t(e, t), Ut(e, t)), o = {}, i;
  ["u", "d"].forEach((a) => {
    i = a === "u" ? t : Object.values(t).reverse(), ["l", "r"].forEach((l) => {
      l === "r" && (i = i.map((f) => Object.values(f).reverse()));
      let d = Qt(e, i, r, (f) => (a === "u" ? e.predecessors(f) : e.successors(f)) || [], n), c = Zt(e, i, d.root, d.align, l === "r");
      l === "r" && (c = X(c, (f) => -f)), o[a + l] = c;
    });
  });
  let s = nr(e, o);
  return tr(o, s), rr(o, e.graph().align);
}
function or(e, n, t) {
  return (r, o, i) => {
    let s = r.node(o), a = r.node(i), l = 0, u;
    if (l += s.width / 2, Object.hasOwn(s, "labelpos")) switch (s.labelpos.toLowerCase()) {
      case "l":
        u = -s.width / 2;
        break;
      case "r":
        u = s.width / 2;
        break;
    }
    if (u && (l += t ? u : -u), u = void 0, l += (s.dummy ? n : e) / 2, l += (a.dummy ? n : e) / 2, l += a.width / 2, Object.hasOwn(a, "labelpos")) switch (a.labelpos.toLowerCase()) {
      case "l":
        u = a.width / 2;
        break;
      case "r":
        u = -a.width / 2;
        break;
    }
    return u && (l += t ? u : -u), l;
  };
}
function ir(e, n) {
  return e.node(n).width;
}
function Nn(e, n, t) {
  var s;
  if (!t) return false;
  let r = (s = n.node(e)) == null ? void 0 : s.edgeObj;
  if (!r || n.node(e).edgeLabel.reversed) return false;
  let o = t.indexOf(r == null ? void 0 : r.v), i = t.indexOf(r == null ? void 0 : r.w);
  return o !== -1 && i !== -1 && o === (i + 1) % t.length || o === (i - 1) % t.length;
}
function kn(e, n) {
  e = Z(e), sr(e), Object.entries(vn(e, n)).forEach(([t, r]) => e.node(t).x = r);
}
function sr(e) {
  let n = P(e), t = e.graph(), r = t.ranksep, o = t.rankalign, i = 0;
  n.forEach((s) => {
    let a = s.reduce((l, u) => {
      var c;
      let d = (c = e.node(u).height) != null ? c : 0;
      return l > d ? l : d;
    }, 0);
    s.forEach((l) => {
      let u = e.node(l);
      o === "top" ? u.y = i + u.height / 2 : o === "bottom" ? u.y = i + a - u.height / 2 : u.y = i + a / 2;
    }), i += a + r;
  });
}
var xn = /* @__PURE__ */ new WeakMap();
function Oe(e, n = {}) {
  return In(e, q, n), e;
}
function _n(e, n, t) {
  let r = n;
  for (; r !== void 0; ) {
    let o = e.parent(r);
    if (o === t) return r;
    r = o;
  }
}
function In(e, n, t) {
  var y;
  let r = e.nodes().filter((b) => e.children(b).length), o = {};
  r.forEach((b) => {
    let p = e.node(b);
    if (p && p.rankdir) {
      let E = new T({ multigraph: true, compound: true });
      E.setGraph({ rankdir: p.rankdir });
      let w = e.children(b);
      w.forEach((G) => {
        let N = { ...e.node(G) };
        E.setNode(G, N);
        let C = e.parent(G);
        C && C !== b && w.includes(C) && E.setParent(G, C);
      });
      let k = /* @__PURE__ */ new Set();
      e.edges().forEach((G) => {
        let N = _n(e, G.v, b), C = _n(e, G.w, b);
        if (N === G.v && C === G.w) E.setEdge(G, { ...e.edge(G) });
        else if (N && C && N !== C) {
          let B = `${N}\0${C}`;
          k.has(B) || (k.add(B), E.setEdge(N, C, { ...e.edge(G) }));
        }
      }), In(E, n, t);
      let _ = Rn(E);
      Cn(_, n, t, null), Tn(E, _);
      let O = 1 / 0, j = 1 / 0, R = -1 / 0, I = -1 / 0;
      E.nodes().forEach((G) => {
        if (G === b) return;
        let N = E.node(G);
        N && typeof N.x == "number" && typeof N.y == "number" && typeof N.width == "number" && typeof N.height == "number" && (O = Math.min(O, N.x - N.width / 2), R = Math.max(R, N.x + N.width / 2), j = Math.min(j, N.y - N.height / 2), I = Math.max(I, N.y + N.height / 2));
      }), (!isFinite(O) || !isFinite(j) || !isFinite(R) || !isFinite(I)) && (O = j = 0, R = I = 0);
      let v = R - O, x = I - j;
      o[b] = { minX: O, minY: j, maxX: R, maxY: I, width: v, height: x, offsetX: O, offsetY: j }, p._dagreClusterSubgraph = E;
    }
  });
  let i = [], s = (b) => {
    let p = [], E = (e.children(b) || []).filter((w) => w !== b);
    for (; E.length > 0; ) {
      let w = E.shift();
      p.push(w), (e.children(w) || []).filter((k) => k !== w).forEach((k) => E.push(k));
    }
    return p;
  }, a = /* @__PURE__ */ new Map();
  r.forEach((b) => {
    let p = e.node(b);
    p && p.rankdir && o[b] && a.set(b, (e.children(b) || []).filter((E) => E !== b));
  });
  let l = new Set([...a.values()].flat()), u = /* @__PURE__ */ new Map();
  a.forEach((b, p) => {
    l.has(p) || u.set(p, s(p));
  });
  let d = new Set([...u.values()].flat()), c = (b) => {
    for (let [p, E] of u) if (E.includes(b)) return p;
    return b;
  }, f = [];
  e.edges().forEach((b) => {
    (d.has(b.v) || d.has(b.w)) && f.push({ edge: b, label: e.edge(b) });
  });
  let h2 = /* @__PURE__ */ new Map();
  d.forEach((b) => {
    let p = e.parent(b);
    h2.set(b, typeof p == "string" ? p : void 0);
  }), u.forEach((b, p) => {
    let E = e.node(p), w = [];
    b.forEach((O) => {
      let j = e.node(O);
      j && (w.push({ id: O, node: j, parent: h2.get(O) }), e.removeNode(O));
    });
    let k = f.filter(({ edge: O }) => b.includes(O.v) || b.includes(O.w)), _ = o[p];
    E && (i.push({ clusterId: p, subgraph: E._dagreClusterSubgraph, bounds: _, children: b, removedNodes: w, removedEdges: k }), E.width = _.width, E.height = _.height);
  });
  let g = /* @__PURE__ */ new Set();
  f.forEach(({ edge: b, label: p }) => {
    let E = c(b.v), w = c(b.w);
    if (E !== w && e.hasNode(E) && e.hasNode(w)) {
      let k = `${E}\0${w}`;
      g.has(k) || (g.add(k), e.setEdge(E, w, { ...p, width: 0, height: 0 }));
    }
  });
  let m = Rn(e), L = Cn(m, n, t, (y = xn.get(e)) != null ? y : null);
  xn.set(e, L), Tn(e, m), g.forEach((b) => {
    let p = b.indexOf("\0"), E = b.slice(0, p), w = b.slice(p + 1);
    e.hasEdge(E, w) && e.removeEdge(E, w);
  }), i.forEach(({ clusterId: b, subgraph: p, bounds: E, removedNodes: w, removedEdges: k }) => {
    var v, x;
    let _ = e.node(b), O = (v = _ == null ? void 0 : _.x) != null ? v : 0, j = (x = _ == null ? void 0 : _.y) != null ? x : 0, R = (E.minX + E.maxX) / 2, I = (E.minY + E.maxY) / 2;
    w.forEach(({ id: G, node: N, parent: C }) => {
      e.setNode(G, N), C !== void 0 && e.setParent(G, C);
    }), k.forEach(({ edge: G, label: N }) => {
      e.setEdge(G, N);
    }), p.nodes().forEach((G) => {
      if (G === b) return;
      let N = p.node(G), C = e.node(G);
      C && N && typeof N.x == "number" && typeof N.y == "number" && (C.x = O + (N.x - R), C.y = j + (N.y - I));
    }), On(e, p, O - R, j - I), delete _._dagreClusterSubgraph;
  }), r.forEach((b) => {
    var w, k;
    let p = e.node(b), E = o[b];
    if (p && p.rankdir && p._dagreClusterSubgraph && E) {
      let _ = p._dagreClusterSubgraph, O = (w = p.x) != null ? w : 0, j = (k = p.y) != null ? k : 0, R = (E.minX + E.maxX) / 2, I = (E.minY + E.maxY) / 2;
      _.nodes().forEach((v) => {
        if (v === b) return;
        let x = _.node(v), G = e.node(v);
        if (G && x && typeof x.x == "number" && typeof x.y == "number") {
          let N = x.x - R, C = x.y - I;
          G.x = O + N, G.y = j + C;
        }
      }), On(e, _, O - R, j - I), delete p._dagreClusterSubgraph;
    }
  });
}
function On(e, n, t, r) {
  n.edges().forEach((o) => {
    if (!e.hasEdge(o)) return;
    let i = n.edge(o), s = e.edge(o);
    s.points = i.points.map((a) => ({ x: a.x + t, y: a.y + r })), Object.hasOwn(i, "x") && (s.x = i.x + t, s.y = i.y + r);
  });
}
function Cn(e, n, t, r = null) {
  var l, u;
  let o = (t == null ? void 0 : t.useDynamic) !== false, i = o && (l = r == null ? void 0 : r.graph) != null ? l : null, s = o && (u = r == null ? void 0 : r.rawNodes) != null ? u : null;
  n("    makeSpaceForEdgeLabels", () => br(e)), n("    removeSelfEdges", () => Gr(e)), n("    acyclic", () => Ue(e, i)), n("    nestingGraph.run", () => un(e)), n("    rank", () => dn(Z(e))), n("    injectEdgeLabelProxies", () => gr(e)), n("    removeEmptyRanks", () => We(e)), n("    nestingGraph.cleanup", () => fn(e)), n("    normalizeRanks", () => Be(e)), n("    assignRankMinMax", () => pr(e)), n("    removeEdgeLabelProxies", () => mr(e)), n("    normalize.run", () => Ke(e)), n("    parentDummyChains", () => ln(e)), n("    addBorderSegments", () => bn(e)), n("    order", () => re(e, t, s)), n("    insertSelfEdges", () => vr(e)), n("    adjustCoordinateSystem", () => pn(e)), n("    position", () => kn(e, t.corePath)), n("    positionSelfEdges", () => kr(e));
  let a = JSON.parse(JSON.stringify(e._nodes));
  return n("    removeBorderNodes", () => Nr(e)), n("    normalize.undo", () => Qe(e)), n("    fixupEdgeLabelCoords", () => yr(e)), n("    undoCoordinateSystem", () => mn(e)), n("    translateGraph", () => Er(e)), n("    assignNodeIntersects", () => Lr(e)), n("    reversePoints", () => wr(e)), n("    acyclic.undo", () => Je(e)), { graph: e, rawNodes: a };
}
function Tn(e, n) {
  e.nodes().forEach((t) => {
    let r = e.node(t), o = n.node(t);
    r && (r.x = o.x, r.y = o.y, r.order = o.order, r.rank = o.rank, n.children(t).length && (r.width = o.width, r.height = o.height));
  }), e.edges().forEach((t) => {
    let r = e.edge(t), o = n.edge(t);
    r.points = o.points, Object.hasOwn(o, "x") && (r.x = o.x, r.y = o.y);
  }), e.graph().width = n.graph().width, e.graph().height = n.graph().height;
}
var ar = ["nodesep", "edgesep", "ranksep", "marginx", "marginy"];
var dr = { ranksep: 50, edgesep: 20, nodesep: 50, rankdir: "TB", rankalign: "center" };
var lr = ["acyclicer", "ranker", "rankdir", "align", "rankalign"];
var ur = ["width", "height", "rank"];
var jn = { width: 0, height: 0 };
var cr = ["minlen", "weight", "width", "height", "labeloffset"];
var fr = { minlen: 1, weight: 1, width: 0, height: 0, labeloffset: 10, labelpos: "r" };
var hr = ["labelpos"];
function Rn(e) {
  let n = new T({ multigraph: true, compound: true }), t = _e(e.graph());
  return n.setGraph(Object.assign({}, dr, xe(t, ar), W(t, lr))), e.nodes().forEach((r) => {
    let o = _e(e.node(r)), i = xe(o, ur);
    Object.keys(jn).forEach((a) => {
      i[a] === void 0 && (i[a] = jn[a]);
    }), n.setNode(r, i);
    let s = e.parent(r);
    s !== void 0 && n.setParent(r, s);
  }), e.edges().forEach((r) => {
    let o = _e(e.edge(r));
    n.setEdge(r, Object.assign({}, fr, xe(o, cr), W(o, hr)));
  }), n;
}
function br(e) {
  let n = e.graph();
  n.ranksep /= 2, e.edges().forEach((t) => {
    var o;
    let r = e.edge(t);
    r.minlen *= 2, ((o = r.labelpos) != null ? o : "r").toLowerCase() !== "c" && (n.rankdir === "TB" || n.rankdir === "BT" ? r.width += r.labeloffset : r.height += r.labeloffset);
  });
}
function gr(e) {
  e.edges().forEach((n) => {
    let t = e.edge(n);
    if (t.width && t.height) {
      let r = e.node(n.v), i = { rank: (e.node(n.w).rank - r.rank) / 2 + r.rank, e: n };
      M(e, "edge-proxy", i, "_ep");
    }
  });
}
function pr(e) {
  let n = 0;
  e.nodes().forEach((t) => {
    let r = e.node(t);
    r.borderTop && (r.minRank = e.node(r.borderTop).rank, r.maxRank = e.node(r.borderBottom).rank, n = Math.max(n, r.maxRank));
  }), e.graph().maxRank = n;
}
function mr(e) {
  e.nodes().forEach((n) => {
    let t = e.node(n);
    if (t.dummy === "edge-proxy") {
      let r = t;
      e.edge(r.e).labelRank = t.rank, e.removeNode(n);
    }
  });
}
function Er(e) {
  let n = Number.POSITIVE_INFINITY, t = 0, r = Number.POSITIVE_INFINITY, o = 0, i = e.graph(), s = i.marginx || 0, a = i.marginy || 0;
  function l(u) {
    let d = u.x, c = u.y, f = u.width, h2 = u.height;
    n = Math.min(n, d - f / 2), t = Math.max(t, d + f / 2), r = Math.min(r, c - h2 / 2), o = Math.max(o, c + h2 / 2);
  }
  e.nodes().forEach((u) => l(e.node(u))), e.edges().forEach((u) => {
    let d = e.edge(u);
    Object.hasOwn(d, "x") && l(d);
  }), n -= s, r -= a, e.nodes().forEach((u) => {
    let d = e.node(u);
    d.x -= n, d.y -= r;
  }), e.edges().forEach((u) => {
    let d = e.edge(u);
    d.points.forEach((c) => {
      c.x -= n, c.y -= r;
    }), Object.hasOwn(d, "x") && (d.x -= n), Object.hasOwn(d, "y") && (d.y -= r);
  }), i.width = t - n + s, i.height = o - r + a;
}
function Lr(e) {
  e.edges().forEach((n) => {
    if (n.v === n.w) return;
    let t = e.edge(n), r = e.node(n.v), o = e.node(n.w), i, s;
    t.points ? (i = t.points[0], s = t.points[t.points.length - 1]) : (t.points = [], i = o, s = r), t.points.unshift(se(r, i)), t.points.push(se(o, s));
  });
}
function yr(e) {
  e.edges().forEach((n) => {
    let t = e.edge(n);
    if (Object.hasOwn(t, "x")) switch ((t.labelpos === "l" || t.labelpos === "r") && (t.width -= t.labeloffset), t.labelpos) {
      case "l":
        t.x -= t.width / 2 + t.labeloffset;
        break;
      case "r":
        t.x += t.width / 2 + t.labeloffset;
        break;
    }
  });
}
function wr(e) {
  e.edges().forEach((n) => {
    let t = e.edge(n);
    t.reversed && t.points.reverse();
  });
}
function Nr(e) {
  e.nodes().forEach((n) => {
    if (e.children(n).length) {
      let t = e.node(n), r = e.node(t.borderTop), o = e.node(t.borderBottom), i = e.node(t.borderLeft[t.borderLeft.length - 1]), s = e.node(t.borderRight[t.borderRight.length - 1]);
      t.width = Math.abs(s.x - i.x), t.height = Math.abs(o.y - r.y), t.x = i.x + t.width / 2, t.y = r.y + t.height / 2;
    }
  }), e.nodes().forEach((n) => {
    e.node(n).dummy === "border" && e.removeNode(n);
  });
}
function Gr(e) {
  e.edges().forEach((n) => {
    if (n.v === n.w) {
      let t = e.node(n.v);
      t.selfEdges || (t.selfEdges = []), t.selfEdges.push({ e: n, label: e.edge(n) }), e.removeEdge(n);
    }
  });
}
function vr(e) {
  P(e).forEach((t) => {
    let r = 0;
    t.forEach((o, i) => {
      let s = e.node(o);
      typeof s.rank != "number" && (s.rank = 0), s.order = i + r, (s.selfEdges || []).forEach((a) => {
        M(e, "selfedge", { width: a.label.width, height: a.label.height, rank: s.rank, order: i + ++r, e: a.e, edgeLabel: a.label }, "_se"), (!Array.isArray(a.label.points) || a.label.points.length !== 7) && (a.label.points = [{ x: 0, y: -10 }, { x: 0, y: -10 }, { x: 0, y: 0 }, { x: 0, y: 10 }, { x: 0, y: 10 }, { x: 0, y: 0 }, { x: 0, y: 0 }]);
      }), delete s.selfEdges;
    });
  });
}
function kr(e) {
  e.nodes().forEach((n) => {
    let t = e.node(n), r = (o) => typeof o == "number" && isFinite(o);
    if (t.dummy === "selfedge") {
      let o = t, i = e.node(o.e.v), s = r(i == null ? void 0 : i.x) ? i.x : 0, a = r(i == null ? void 0 : i.y) ? i.y : 0, l = r(i == null ? void 0 : i.width) ? i.width : 0, u = r(i == null ? void 0 : i.height) ? i.height : 0, d = r(t.x) ? t.x : s, c = r(t.y) ? t.y : a, f = l / 2, h2 = u / 2;
      o.edgeLabel.points = [{ x: d + f, y: c - h2 }, { x: d + f, y: c - h2 }, { x: d, y: c }, { x: d - f, y: c + h2 }, { x: d - f, y: c + h2 }, { x: d, y: c }, { x: d, y: c }], o.edgeLabel.x = d, o.edgeLabel.y = c, e.setEdge(o.e, o.edgeLabel), e.removeNode(n);
    } else t && Array.isArray(t.selfEdges) && t.selfEdges.forEach((o) => {
      if (!Array.isArray(o.label.points) || o.label.points.length !== 7) {
        let i = r(t.x) ? t.x : 0, s = r(t.y) ? t.y : 0, a = r(t.width) ? t.width : 0, l = r(t.height) ? t.height : 0, u = a / 2, d = l / 2;
        o.label.points = [{ x: i + u, y: s - d }, { x: i + u, y: s - d }, { x: i, y: s }, { x: i - u, y: s + d }, { x: i - u, y: s + d }, { x: i, y: s }, { x: i, y: s }];
      }
    });
  });
}
function xe(e, n) {
  return X(W(e, n), Number);
}
function _e(e) {
  let n = {};
  return e && Object.entries(e).forEach(([t, r]) => {
    typeof t == "string" && (t = t.toLowerCase()), n[t] = r;
  }), n;
}
function Ce(e) {
  let n = P(e), t = new T({ compound: true, multigraph: true }).setGraph({});
  return e.nodes().forEach((r) => {
    t.setNode(r, { label: r }), t.setParent(r, "layer" + e.node(r).rank);
  }), e.edges().forEach((r) => t.setEdge(r.v, r.w, {}, r.name)), n.forEach((r, o) => {
    let i = "layer" + o;
    t.setNode(i, { rank: "same" }), r.reduce((s, a) => (t.setEdge(s, a, { style: "invis" }), a));
  }), t;
}
var xr = { graphlib: ie, version: ue, layout: Oe, debug: Ce, util: { time: le, notime: q } };
var Uo = xr;

// src/depmap.ts
function needPairs(items, view) {
  const shown = new Set(view);
  const seen = /* @__PURE__ */ new Set();
  const out = [];
  for (const i of view) {
    for (const e of items[i].edges) {
      const t = e.target;
      if (!e.needs || t === null || t === i || !shown.has(t) || seen.has(`${t} ${i}`)) continue;
      seen.add(`${t} ${i}`);
      out.push([t, i]);
    }
  }
  return out;
}
function reach(next, i) {
  const out = /* @__PURE__ */ new Set();
  const queue = [i];
  while (queue.length) {
    for (const j of next.get(queue.pop()) ?? []) {
      if (j === i || out.has(j)) continue;
      out.add(j);
      queue.push(j);
    }
  }
  return out;
}
function adjacency(pairs, up) {
  const out = /* @__PURE__ */ new Map();
  for (const [from, to] of pairs) {
    const [a, b] = up ? [to, from] : [from, to];
    out.set(a, [...out.get(a) ?? [], b]);
  }
  return out;
}
function cascadeOf(items, view, i) {
  return reach(adjacency(needPairs(items, view), false), i);
}
function groundOf(items, view, i) {
  return reach(adjacency(needPairs(items, view), true), i);
}
function depMap(items, view, i, node) {
  const keep = /* @__PURE__ */ new Set([i, ...groundOf(items, view, i), ...cascadeOf(items, view, i)]);
  const pairs = needPairs(items, view).filter(([from, to]) => keep.has(from) && keep.has(to));
  const g = new Uo.graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: node.height * 0.3, ranksep: node.width * 0.35, marginx: 8, marginy: 8 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const k of view) if (keep.has(k)) g.setNode(String(k), { ...node });
  for (const [from, to] of pairs) g.setEdge(String(from), String(to));
  Uo.layout(g);
  const nodes = /* @__PURE__ */ new Map();
  for (const id of g.nodes()) {
    const n = g.node(id);
    nodes.set(Number(id), { x: n.x, y: n.y });
  }
  const edges = pairs.map(([from, to]) => ({
    from,
    to,
    points: g.edge(String(from), String(to)).points ?? [],
    outOfOrder: items[to].reviewed && !items[from].reviewed
  }));
  const { width = 0, height = 0 } = g.graph();
  return { nodes, edges, width, height };
}
function smoothPath(ps) {
  if (!ps.length) return "";
  let d = `M ${ps[0].x} ${ps[0].y}`;
  for (let k = 1; k < ps.length - 1; k++) {
    const mid = { x: (ps[k].x + ps[k + 1].x) / 2, y: (ps[k].y + ps[k + 1].y) / 2 };
    d += ` Q ${ps[k].x} ${ps[k].y} ${mid.x} ${mid.y}`;
  }
  const last = ps[ps.length - 1];
  return `${d} L ${last.x} ${last.y}`;
}

// src/palette.ts
function parseCommand(line) {
  const t = line.trim().replace(/^:/, "").trimStart();
  const sp = t.search(/\s/);
  return sp < 0 ? { name: t, arg: "" } : { name: t.slice(0, sp), arg: t.slice(sp + 1).trim() };
}
var CLIENT_COMMANDS = [
  { name: "theme", args: "<name>", help: "switch the colour theme (auto follows the system)" },
  { name: "map", args: "", help: "the hunk's whole chain: what it stands on to its left, what rejecting it strands to its right" },
  { name: "summary", args: "", help: "the review on one page: progress, what you wrote, what ordo flagged; again to go back" },
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

// src/summary.ts
function files(r) {
  const by = /* @__PURE__ */ new Map();
  for (const i of r.view) {
    const it2 = r.items[i];
    const f = by.get(it2.path) ?? { path: it2.path, hunks: [], read: 0, comments: 0, levels: {} };
    f.hunks.push(i);
    if (it2.reviewed) f.read++;
    for (const x of it2.findings) f.levels[x.level] = (f.levels[x.level] ?? 0) + 1;
    by.set(it2.path, f);
  }
  for (const c of r.comments) {
    const f = by.get(c.path);
    if (f) f.comments++;
  }
  return [...by.values()];
}
function hunkAt(r, path, line) {
  let best = null;
  for (const i of r.view) {
    const it2 = r.items[i];
    if (it2.path !== path) continue;
    if (it2.new[0] <= line && line <= it2.new[1]) return i;
    if (it2.new[0] <= line && (best === null || it2.new[0] > r.items[best].new[0])) best = i;
  }
  return best;
}
function written(r) {
  const out = [];
  const pos = new Map(r.view.map((i, k) => [i, k]));
  const fileAt = /* @__PURE__ */ new Map();
  for (const i of r.view) if (!fileAt.has(r.items[i].path)) fileAt.set(r.items[i].path, pos.get(i));
  const seen = /* @__PURE__ */ new Set();
  for (const i of r.view) {
    const it2 = r.items[i];
    const key = `${it2.symbol || it2.symbols[0] || i}\0${it2.note}`;
    if (!it2.note || seen.has(key)) continue;
    seen.add(key);
    out.push({ at: [fileAt.get(it2.path), it2.new[0]], w: { kind: "note", i, text: it2.note } });
  }
  for (const c of r.comments) {
    const file = fileAt.get(c.path);
    if (file === void 0) continue;
    out.push({ at: [file, c.start], w: { kind: "comment", i: hunkAt(r, c.path, c.start), comment: c } });
  }
  return out.sort((a, b) => a.at[0] - b.at[0] || a.at[1] - b.at[1]).map((x) => x.w);
}
function flagged(r, levels) {
  const out = r.view.flatMap((i) => r.items[i].findings.filter((f) => levels.includes(f.level)).map((f) => ({ i, f })));
  return out.sort((a, b) => levels.indexOf(a.f.level) - levels.indexOf(b.f.level));
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
var st2 = {
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
  notice: "",
  /** the one-page summary stands in for the columns */
  summary: false
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
    st2.status = String(e.message);
    return renderAll();
  }
  if (seq !== refreshSeq) return;
  if (slot.state !== "ready") {
    st2.status = slot.status;
    st2.empty = slot.state === "empty";
    st2.review = null;
    return renderAll();
  }
  st2.empty = false;
  if (slot.version < st2.version && slot.generation === st2.review?.generation) return;
  const prev = st2.review;
  st2.review = slot;
  st2.version = slot.version;
  st2.keymap = new Keymap(slot.keys);
  const moved = !!prev && prev.generation !== slot.generation;
  const remap = (i) => {
    if (!prev || !moved) return i;
    const id = identity(prev.items[i]);
    return id ? slot.items.findIndex((it2) => identity(it2) === id) : -1;
  };
  let sel = st2.sel >= 0 ? remap(st2.sel) : -1;
  if (sel < 0) sel = prev ? clampToView(Math.min(st2.sel, slot.items.length - 1)) : slot.sel;
  if (moved) {
    st2.jumps = st2.jumps.map((j) => ({ ...j, sel: remap(j.sel) })).filter((j) => j.sel >= 0);
    if (depmap) {
      depmap.root = remap(depmap.root);
      depmap.sel = remap(depmap.sel);
      if (depmap.root < 0) closeSheet();
    }
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
  if (slot.delta.new + slot.delta.changed + slot.delta.moved === 0) st2.sinceOnly = false;
  if (prev?.version !== slot.version) {
    st2.files.clear();
    filesGen++;
    st2.facts = null;
  }
  const same = sel === st2.sel && !!st2.file;
  if (same) st2.sel = -1;
  await select(sel, { keepCursor: same });
  if (canvas) renderCanvas();
  if (depmap) renderMap(true);
}
function identity(it2) {
  return it2 ? `${it2.path}\0${it2.new[0]}\0${it2.rationale}` : "";
}
function clampToView(i) {
  const v = st2.review.view;
  return v.includes(i) ? i : v[0] ?? 0;
}
async function fileOf(path) {
  const hit = st2.files.get(path);
  if (hit) return hit;
  const gen = filesGen;
  const f = await api.file(path);
  if (gen === filesGen) st2.files.set(path, f);
  return f;
}
var factsFor = -1;
async function loadFacts(i) {
  factsFor = i;
  const version = st2.version;
  try {
    const f = await api.item(i);
    if (factsFor === i && version === st2.version) {
      st2.facts = f;
      renderWhy();
      renderBand();
    }
  } catch {
  }
}
async function select(i, opts = {}) {
  const r = st2.review;
  if (!r || i < 0 || i >= r.items.length) return renderAll();
  const seq = ++selectSeq;
  const it2 = r.items[i];
  let f;
  try {
    f = await fileOf(it2.path);
  } catch (e) {
    if (seq === selectSeq) toast(e.message, true);
    return;
  }
  if (seq !== selectSeq || st2.review !== r) return;
  const changed = i !== st2.sel;
  if (changed && st2.file?.path !== it2.path) st2.search = null;
  st2.sel = i;
  if (opts.focus) st2.focus = opts.focus;
  st2.file = f.missing ? null : f;
  st2.rows = st2.file ? codeRows(st2.file) : [];
  if (!opts.keepCursor) {
    st2.col = null;
    st2.cursor = firstRowOf(st2.rows, i);
    st2.from = null;
  }
  st2.cursor = Math.min(st2.cursor, Math.max(0, st2.rows.length - 1));
  if (changed) {
    st2.facts = null;
    st2.whyCur = -1;
  }
  renderAll();
  scrollCodeTo(firstRowOf(st2.rows, i), !opts.keepCursor || changed);
  loadFacts(i);
}
async function act(body) {
  try {
    const out = await api.act({
      sel: st2.sel,
      generation: st2.review?.generation,
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
      const q2 = new URLSearchParams({ rev: out.navigate });
      location.href = `/?${q2}#t=${token}`;
    }
    if (out.version > st2.version) await refresh();
    if (out.sel !== void 0 && out.sel !== st2.sel) await select(out.sel);
    renderConfig();
    return out;
  } catch (e) {
    toast(e.message, true);
    refresh();
    return null;
  }
}
function lineOfCursor() {
  const row = st2.rows[st2.cursor];
  if (!row) return void 0;
  if (row.kind !== "del") return row.n - 1;
  const next = st2.rows.slice(st2.cursor).find((r) => r.kind !== "del");
  return next ? next.n - 1 : void 0;
}
function fromLine() {
  if (st2.from === null) return void 0;
  const row = st2.rows[st2.from];
  return row && row.kind !== "del" ? row.n - 1 : void 0;
}
function shownRows() {
  const r = st2.review;
  if (!st2.sinceOnly) return r.rows;
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
    (row) => row.kind === "item" ? [row.i] : row.kind === "folded" ? [row.items.includes(st2.sel) ? st2.sel : row.items[0]] : []
  );
}
function step(by) {
  const s = stops();
  if (!s.length) return;
  const at2 = s.indexOf(st2.sel);
  const to = at2 < 0 ? 0 : Math.max(0, Math.min(s.length - 1, at2 + by));
  select(s[to]);
}
function rowClass(items, selected) {
  const r = st2.review;
  const its = items.map((i) => r.items[i]);
  const reviewed = its.every((it2) => it2.reviewed);
  const level = LEVELS.find((l) => its.some((it2) => it2.level === l));
  const quiet = reviewed || its.every((it2) => it2.noise);
  return ["r", reviewed && "reviewed", level && `lv-${level}`, selected && "sel", quiet && "quiet"].filter(Boolean).join(" ");
}
function renderList() {
  const pane = $("#list");
  const r = st2.review;
  const scope = [
    `${r.view.length} hunk${r.view.length === 1 ? "" : "s"}`,
    r.mode === "ledger" && "by symbol",
    st2.sinceOnly && "changed since your last look",
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
      const selected = items.includes(st2.sel);
      const folded = row.kind === "folded";
      const level = LEVELS.find((l) => items.some((i) => r.items[i].level === l));
      const el = h(
        "div",
        {
          class: `${rowClass(items, selected)}${folded ? " folded" : ""}`,
          title: folded ? first.symbol ?? "" : first.rationale,
          onclick: () => {
            st2.focus = "list";
            select(items.includes(st2.sel) ? st2.sel : items[0]);
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
  const it2 = st2.review.items[st2.sel];
  const from = dots.get(st2.sel);
  if (!it2 || !from) return;
  const rail = parseFloat(getComputedStyle(rows).getPropertyValue("--rail")) || 30;
  const mid = (el) => el.offsetTop + el.offsetHeight / 2;
  const y0 = mid(from);
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "arcs");
  svg.setAttribute("height", String(rows.offsetHeight));
  svg.setAttribute("aria-hidden", "true");
  const arcs = /* @__PURE__ */ new Map();
  for (const e of it2.edges) {
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
  const top = el ? el.offsetTop : st2.cursor * rh;
  if (top < body.scrollTop) body.scrollTop = top;
  else if (top + rh > body.scrollTop + body.clientHeight) body.scrollTop = top + rh - body.clientHeight;
}
function lineEl(row, k, mine, findings, uses, commented) {
  const search = st2.search;
  const caret = k === st2.cursor ? caretCol(row.text) : -1;
  let at2 = 0;
  const parts = runs(row.text, row.spans, row.marks, row.kind === "edit" ? row.gone : []).flatMap((run2) => {
    if (run2.gone) return [h("del", { class: "gone", title: "removed" }, run2.text)];
    const piece = (text, extra) => h(run2.changed ? "mark" : "span", { class: [run2.role && `s-${run2.role}`, extra].filter(Boolean).join(" ") || null }, text);
    const cs = [...run2.text];
    const i = caret - at2;
    at2 += cs.length;
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
    k === st2.cursor && "cur",
    st2.from !== null && k >= Math.min(st2.from, st2.cursor) && k <= Math.max(st2.from, st2.cursor) && "picked",
    row.kind !== "del" && commented.has(row.n) && "commented"
  ].filter(Boolean).join(" ");
  const out = [
    h(
      "div",
      {
        class: cls,
        dataset: { k: String(k) },
        onclick: (e) => {
          st2.cursor = k;
          st2.col = caretColumn(e, row.text);
          st2.focus = "code";
          if (row.hunk !== null && row.hunk !== st2.sel) select(row.hunk, { keepCursor: true, focus: "code" });
          else renderCode();
        },
        onmousemove: (e) => row.kind !== "del" && peekLater(e, row.n, row.text),
        onmouseleave: () => peekLeave()
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
  const q2 = s.query.toLowerCase();
  let at2 = 0;
  for (let p = low.indexOf(q2); q2 && p >= 0; p = low.indexOf(q2, p + q2.length)) {
    out.push(document.createTextNode(text.slice(at2, p)));
    out.push(h("span", { class: `hit${s.hits[s.k] === k ? " now" : ""}` }, text.slice(p, p + q2.length)));
    at2 = p + q2.length;
  }
  out.push(document.createTextNode(text.slice(at2)));
  return out;
}
function renderReading(it2) {
  const r = st2.review;
  const head = $("#code > .head");
  const actions = $("#code .foot .actions");
  if (!it2) {
    replace(head);
    replace(actions);
    return;
  }
  const pos = r.view.indexOf(st2.sel);
  const what = it2.what.length ? it2.what : ["a change ordo found nothing to say about"];
  const search = st2.search;
  replace(
    head,
    h(
      "div",
      { class: "kicker" },
      h("span", null, pos >= 0 ? `Hunk ${pos + 1} of ${r.view.length}` : "Outside the current filter"),
      h("span", { class: "path" }, `${it2.path}:${it2.new[0]}`),
      h("span", { class: "grow" }),
      zoomButton("code")
    ),
    h("h1", { class: "headline" }, prose(sentence(what[0]))),
    (what.length > 1 || it2.details.length > 0 || it2.grouped_by) && h(
      "div",
      { class: "lede" },
      what.slice(1).map((w) => h("p", null, prose(sentence(w)))),
      it2.details.map((d) => h("p", { class: "detail" }, prose(sentence(d)))),
      it2.grouped_by && h("p", null, "Grouped by ", prose(it2.grouped_by))
    ),
    h(
      "div",
      { class: "pills" },
      it2.reviewed && h("span", { class: "pill done" }, "Read"),
      it2.delta && h("span", { class: "pill accent" }, SINCE[it2.delta][0], sentence(SINCE[it2.delta][1].split(" \u2014")[0])),
      it2.noise && !/formatting/.test(it2.rationale) && h("span", { class: "pill" }, "Quiet: generated or formatting"),
      it2.wave !== null && h("span", { class: "pill" }, `Wave ${it2.wave}`)
    ),
    (search || st2.from !== null) && h(
      "div",
      { class: "status" },
      search && h("span", null, `Searching for \u201C${search.query}\u201D: ${search.hits.length ? `${search.k + 1} of ${search.hits.length}` : "no match"}`),
      st2.from !== null && h("span", null, "Selecting lines; press c to comment on them")
    )
  );
  const key = st2.keymap.keysFor("toggle-reviewed")[0];
  const s = stops();
  const at2 = s.indexOf(st2.sel);
  const next = at2 >= 0 && at2 + 1 < s.length ? r.items[s[at2 + 1]] : null;
  replace(
    actions,
    h(
      "button",
      { class: `read${it2.reviewed ? " done" : ""}`, onclick: () => run("toggle-reviewed"), title: it2.reviewed ? "Mark as unread" : "Mark as read and go to the next hunk" },
      it2.reviewed ? "Marked as read" : "Mark as read",
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
  let at2 = 0;
  for (const m of text.matchAll(NAME)) {
    if (m.index > at2) out.push(text.slice(at2, m.index));
    out.push(h("span", { class: "code-word" }, m[0].replace(/^`|`$/g, "")));
    at2 = m.index + m[0].length;
  }
  if (at2 < text.length) out.push(text.slice(at2));
  return out;
}
function renderCode() {
  const pane = $("#code");
  const r = st2.review;
  const it2 = r.items[st2.sel];
  renderReading(it2);
  const body = pane.querySelector(".body");
  if (!st2.file || !it2) return replace(body);
  const findings = /* @__PURE__ */ new Map();
  for (const f of it2.findings) {
    if (f.row == null) continue;
    findings.set(f.row, [...findings.get(f.row) ?? [], f]);
  }
  const uses = new Set(it2.uses_at.flatMap((u) => u.rows));
  const commented = new Set(st2.file.comments.flatMap((c) => range(c.start, c.end)));
  const code = h("div", { class: "code" });
  st2.rows.forEach((row, k) => {
    for (const el of lineEl(row, k, row.hunk === st2.sel, findings, uses, commented)) code.append(el);
    if (row.kind !== "del") {
      for (const c of st2.file.comments.filter((c2) => c2.end === row.n)) {
        code.append(h("div", { class: "cmt" }, `\u270E ${c.text}${c.stale ? " (lines changed since)" : ""}`));
      }
    }
  });
  const top = body.scrollTop;
  replace(body, code);
  body.scrollTop = top;
  renderStrip();
}
function renderStrip() {
  const pane = $("#code");
  const body = pane.querySelector(".body");
  let strip = pane.querySelector(".strip");
  if (!strip) {
    strip = h("div", { class: "strip", title: "Where this file changed, and what sits on its lines. Click to go there." });
    const go = (e) => {
      const rect = strip.getBoundingClientRect();
      const frac = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height));
      body.scrollTop = frac * body.scrollHeight - body.clientHeight / 2;
    };
    strip.addEventListener("mousedown", (e) => {
      e.preventDefault();
      go(e);
      const move = (m) => go(m);
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", () => window.removeEventListener("mousemove", move), { once: true });
    });
    body.addEventListener("scroll", stripView);
    pane.append(strip);
  }
  const r = st2.review;
  if (!r || !st2.file || !st2.rows.length) {
    strip.hidden = true;
    return;
  }
  strip.hidden = false;
  strip.style.top = `${body.offsetTop}px`;
  strip.style.height = `${body.clientHeight}px`;
  strip.style.right = `${body.offsetWidth - body.clientWidth}px`;
  const findings = /* @__PURE__ */ new Map();
  for (const it2 of r.items) {
    if (it2.path !== st2.file.path) continue;
    for (const f of it2.findings) {
      if (f.row == null || f.level === "note" || findings.get(f.row) === "verdict") continue;
      findings.set(f.row, f.level);
    }
  }
  const comments = new Set(st2.file.comments.flatMap((c) => range(c.start, c.end)));
  const n = st2.rows.length;
  replace(
    strip,
    stripMarks(st2.rows, st2.sel, findings, comments, st2.search?.hits ?? []).map(
      (m) => h("i", { class: m.kind, style: `top:${100 * m.from / n}%;height:max(3px, ${100 * (m.to - m.from + 1) / n}%)` })
    ),
    h("b", { class: "view" })
  );
  stripView();
}
function stripView() {
  const body = $("#code .body");
  const view = $("#code .strip").querySelector(".view");
  if (!view || !body.scrollHeight) return;
  view.style.top = `${100 * body.scrollTop / body.scrollHeight}%`;
  view.style.height = `${100 * body.clientHeight / body.scrollHeight}%`;
}
var PEEK_DELAY = 450;
var peekTimer = 0;
var peekSeq = 0;
var peekKey = "";
function peekLater(e, line, text) {
  const col = caretColumn(e, text);
  const w = col === null ? null : wordAt(text, col);
  const key = w ? `${line}:${w.start}` : "";
  if (key === peekKey) return;
  peekKey = key;
  clearTimeout(peekTimer);
  peekSeq++;
  if (!$("#peek").matches(":hover")) hidePeek();
  if (!w) return;
  const at2 = { x: e.clientX, y: e.clientY };
  peekTimer = window.setTimeout(() => showPeek(line, w.word, w.start, at2), PEEK_DELAY);
}
function peekLeave() {
  clearTimeout(peekTimer);
  peekSeq++;
  peekKey = "";
  setTimeout(() => {
    if (!$("#peek").matches(":hover")) hidePeek();
  }, 150);
}
function hidePeek() {
  const p = $("#peek");
  p.hidden = true;
  replace(p);
}
async function showPeek(line, word, col, at2) {
  const r = st2.review;
  if (!r) return;
  const seq = ++peekSeq;
  const changed = r.view.filter((i) => i !== st2.sel && (r.items[i].symbols.includes(word) || r.items[i].enclosing === word));
  let found = [];
  try {
    const out = await api.act({ op: "hover", brief: true, sel: st2.sel, generation: r.generation, line: line - 1, col });
    found = out.popup?.lines ?? [];
  } catch {
  }
  if (seq !== peekSeq) return;
  if (!found.length && !changed.length) return;
  const shown = changed.slice(0, 3);
  const p = $("#peek");
  replace(
    p,
    h("header", null, word),
    found.length ? h("pre", null, found.join("\n").trim()) : null,
    changed.length ? h(
      "div",
      { class: "changed" },
      h("h4", null, changed.length === 1 ? "Changed in this review" : `Changed in ${changed.length} hunks of this review`),
      shown.map(
        (i) => h(
          "button",
          { onclick: () => (hidePeek(), jumpTo(i)) },
          h("span", { class: "at" }, `${r.items[i].path}:${r.items[i].new[0]}`),
          h("span", null, sentence(r.items[i].rationale))
        )
      ),
      changed.length > shown.length ? h("p", { class: "more" }, `and ${changed.length - shown.length} more`) : null
    ) : null
  );
  p.hidden = false;
  const w = p.offsetWidth;
  const ht2 = p.offsetHeight;
  p.style.left = `${Math.max(8, Math.min(at2.x + 12, innerWidth - w - 8))}px`;
  p.style.top = `${at2.y + 18 + ht2 < innerHeight ? at2.y + 18 : Math.max(8, at2.y - ht2 - 12)}px`;
}
function caretColumn(e, text) {
  const doc = document;
  const pos = doc.caretPositionFromPoint?.(e.clientX, e.clientY);
  const line = e.currentTarget.lastElementChild;
  if (!pos || !line || !line.contains(pos.offsetNode)) return null;
  const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
  let at2 = 0;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.parentElement?.closest(".gone")) {
      if (n === pos.offsetNode) return Math.min(at2, [...text].length);
      continue;
    }
    if (n === pos.offsetNode) return Math.min(at2 + [...(n.textContent ?? "").slice(0, pos.offset)].length, [...text].length);
    at2 += [...n.textContent ?? ""].length;
  }
  return null;
}
function caretCol(text) {
  if (st2.col === null) return -1;
  return Math.min(st2.col, Math.max(0, [...text].length - 1));
}
function firstWordCol(text) {
  const m = /[A-Za-z_]/.exec(text);
  return m ? [...text.slice(0, m.index)].length : 0;
}
function openEditor() {
  const r = st2.review;
  const it2 = r.items[st2.sel];
  if (!it2) return;
  const row = st2.rows[st2.cursor];
  const line = row && row.kind !== "del" ? row.n : it2.new[0];
  if (r.editor === "server") {
    act({ op: "open-editor" });
    return;
  }
  const segments = (p) => p.split("/").map(encodeURIComponent).join("/");
  const url = (r.editor || "vscode://file/{root}/{path}:{line}").replaceAll("{root}", segments(r.root).replace(/^\//, "")).replaceAll("{path}", segments(it2.path)).replaceAll("{line}", String(line));
  window.open(url, "_self");
}
function range(a, b) {
  return Array.from({ length: Math.max(0, b - a + 1) }, (_, k) => a + k);
}
function moveCursor(by) {
  cursorToRow(Math.max(0, Math.min(st2.rows.length - 1, st2.cursor + by)));
}
function cursorToRow(k) {
  st2.cursor = k;
  const hunk = st2.rows[st2.cursor]?.hunk;
  if (hunk !== null && hunk !== void 0 && hunk !== st2.sel) {
    select(hunk, { keepCursor: true }).then(followCursor);
    return;
  }
  renderCode();
  followCursor();
}
function cursorTo(pred, forward) {
  const n = st2.rows.length;
  for (let d = 1; d < n; d++) {
    const k = forward ? st2.cursor + d : st2.cursor - d;
    if (k < 0 || k >= n) break;
    if (pred(st2.rows[k], k)) {
      st2.cursor = k;
      renderCode();
      followCursor();
      return;
    }
  }
}
function edgeRows(targets, limit) {
  const r = st2.review;
  return targets.slice(0, limit).map(({ target, label }) => {
    if (target === null || !r.items[target]) {
      return h("div", { class: "link out" }, h("span", { class: "who" }, label), h("span", { class: "what" }, "Not part of this review"));
    }
    const t = r.items[target];
    const link = st2.whyLinks.length;
    st2.whyLinks.push(target);
    return h(
      "div",
      {
        class: `link${st2.whyCur === link ? " cur" : ""}`,
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
  const limit = tableLimit(targets.length, st2.zoom && st2.focus === "why");
  const more = targets.length - limit;
  return [
    h("h3", null, title, hint && h("small", null, hint)),
    targets.length ? h("div", { class: "links" }, edgeRows(targets, limit)) : h("p", { class: "muted" }, empty),
    more > 0 && h(
      "button",
      {
        class: "more",
        onclick: () => {
          st2.focus = "why";
          st2.zoom = true;
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
  const r = st2.review;
  const it2 = r.items[st2.sel];
  replace(pane.querySelector(".head"), h("span", { class: "t" }, "Why it is here"), h("span", { class: "grow" }), zoomButton("why"));
  const body = pane.querySelector(".body");
  if (!it2) return replace(body);
  st2.whyLinks = [];
  const facts = st2.facts && st2.facts.i === st2.sel ? st2.facts : null;
  const needs = it2.edges.filter((e) => e.needs);
  const neededBy = it2.edges.filter((e) => !e.needs);
  const findings = [...it2.findings].sort((a, b) => LEVELS.indexOf(a.level) - LEVELS.indexOf(b.level));
  const churn = r.file_churn[it2.path];
  const lc = facts?.line_churn;
  const top = body.scrollTop;
  replace(
    body,
    h(
      "div",
      { class: "why" },
      it2.note && [h("h3", null, "Your note"), h("p", { class: "warnline" }, it2.note)],
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
      it2.executed && [
        h("h3", null, "Test coverage"),
        h(
          "p",
          { class: it2.executed[0] === it2.executed[1] ? "" : "warnline" },
          it2.executed[0] === it2.executed[1] ? `All ${it2.executed[1]} executable lines here run under a test.` : `${it2.executed[1] - it2.executed[0]} of ${it2.executed[1]} executable lines here run under no test.`
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
      it2.notes.length > 0 && [h("h3", null, "Also"), it2.notes.map((n) => h("p", null, prose(sentence(n))))],
      it2.wave !== null && [
        h("h3", null, "Wave"),
        h("p", null, `Wave ${it2.wave}${r.wave_intents[String(it2.wave)] ? `, asked for: ${r.wave_intents[String(it2.wave)]}` : ""}`)
      ]
    )
  );
  body.scrollTop = top;
}
function cursorToLine(n) {
  const k = st2.rows.findIndex((row) => row.kind !== "del" && row.n === n);
  if (k < 0) return;
  st2.cursor = k;
  st2.focus = "code";
  renderAll();
  followCursor();
}
function jumpTo(target) {
  st2.jumps.push({ sel: st2.sel, cursor: st2.cursor });
  select(target, { focus: "code" });
}
function jumpBack() {
  const j = st2.jumps.pop();
  if (!j) return;
  select(j.sel, { focus: "code" }).then(() => {
    st2.cursor = Math.min(j.cursor, st2.rows.length - 1);
    renderCode();
    followCursor();
  });
}
function renderBand() {
  const band = $("#band");
  const r = st2.review;
  const help = h("button", { class: "sig help", onclick: () => showHelp(), title: "Every key and command" }, "Keys");
  if (!r) {
    return replace(
      band,
      h("span", { class: "mark" }, "ordo"),
      h("span", { class: "rev" }, rev),
      h("span", { class: "subject" }, st2.status),
      st2.empty && h("button", { class: "sig", onclick: () => act({ op: "reload" }) }, "Read again")
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
        "aria-pressed": st2.sinceOnly ? "true" : "false",
        title: `${r.delta.new} new, ${r.delta.changed} changed, ${r.delta.moved} moved, ${r.delta.gone} gone. Show only these.`,
        onclick: () => {
          st2.sinceOnly = !st2.sinceOnly;
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
      "button",
      {
        class: "sig",
        "aria-pressed": st2.summary ? "true" : "false",
        title: `${r.coverage.both} of ${r.coverage.edges} dependency links have both ends read. The review on one page (:summary).`,
        onclick: () => toggleSummary()
      },
      h("span", { class: "meter" }, h("i", { style: `width:${r.coverage.total ? 100 * r.coverage.done / r.coverage.total : 0}%` })),
      h("b", null, r.coverage.done),
      `of ${r.coverage.total} read`
    ),
    help
  );
}
function toggleSummary() {
  st2.summary = !st2.summary;
  renderAll();
  if (st2.summary) $("#summary").focus();
}
function summaryKey(action) {
  const page2 = $("#summary");
  const by = { next: 60, prev: -60, "half-down": 0.5, "half-up": -0.5, "page-down": 1, "page-up": -1 };
  if (action in by) {
    page2.scrollTop += Math.abs(by[action]) < 2 ? by[action] * page2.clientHeight : by[action];
  } else if (action === "first" || action === "last") {
    page2.scrollTop = action === "first" ? 0 : page2.scrollHeight;
  } else if (action === "quit") {
    if (!dismissSheet()) toggleSummary();
  } else {
    return !["command", "command-goto", "help", "reload"].includes(action);
  }
  return true;
}
function readFromSummary(i) {
  st2.summary = false;
  jumpTo(i);
}
function renderSummary() {
  const r = st2.review;
  const page2 = $("#summary");
  const keep = page2.scrollTop;
  const fl = files(r);
  const notes = written(r);
  const serious = flagged(r, ["verdict", "warn"]);
  const n = (k, one, many = `${one}s`) => `${k} ${k === 1 ? one : many}`;
  const at2 = (path, line, end) => `${path}:${line}${end && end > line ? `\u2013${end}` : ""}`;
  const link = (i, label) => i === null ? h("span", { class: "at" }, label) : h("button", { class: "at", title: "Read this hunk", onclick: () => readFromSummary(i) }, label);
  const nComments = notes.filter((w) => w.kind === "comment").length;
  const nNotes = notes.length - nComments;
  const unreadFiles = fl.filter((f) => f.read < f.hunks.length).length;
  const count = (l) => serious.filter((x) => x.f.level === l).length;
  const notesLeft = flagged(r, ["note"]).length;
  const minor = notesLeft ? h("p", { class: "aside" }, `ordo also left ${n(notesLeft, "note")} on hunks; they are in each hunk's margin.`) : null;
  replace(
    page2,
    h(
      "article",
      null,
      r.subject ? h("p", { class: "kicker" }, h("span", { class: "path" }, r.rev)) : null,
      h("h1", { class: "headline" }, r.subject ?? (r.uncommitted ? "Uncommitted work" : `Review of ${r.rev}`)),
      h(
        "p",
        { class: "lede" },
        `You have read ${r.coverage.done} of ${n(r.coverage.total, "hunk")}`,
        unreadFiles ? `, with ${n(unreadFiles, "file")} still to finish` : ", every file",
        `. You wrote ${n(nComments, "comment")} and ${n(nNotes, "note")}; ordo flagged ${n(count("verdict"), "verdict")} and ${n(count("warn"), "warning")}.`
      ),
      h("div", { class: "meter wide" }, h("i", { style: `width:${r.coverage.total ? 100 * r.coverage.done / r.coverage.total : 0}%` })),
      h(
        "div",
        { class: "actions" },
        h("button", { class: "primary", title: "Your notes and comments as one markdown message (:yank)", onclick: () => act({ op: "command", line_text: "yank" }) }, "Copy for the author"),
        h("button", { title: "The same, with ordo's own notes and findings (:yank all)", onclick: () => act({ op: "command", line_text: "yank all" }) }, "Copy with ordo's findings"),
        h("button", { onclick: () => window.print() }, "Print"),
        h("button", { onclick: () => toggleSummary() }, "Back to reading")
      ),
      h("h2", null, "What you wrote"),
      notes.length ? h(
        "ol",
        { class: "written" },
        notes.map((w) => {
          if (w.kind === "note") {
            const it2 = r.items[w.i];
            return h("li", null, h("div", { class: "where" }, link(w.i, at2(it2.path, it2.new[0])), h("span", null, `note on ${it2.symbol || it2.symbols[0] || "this hunk"}`)), h("p", { class: "said" }, w.text));
          }
          const c = w.comment;
          return h(
            "li",
            null,
            h("div", { class: "where" }, link(w.i, at2(c.path, c.start, c.end)), c.stale ? h("span", { class: "stale" }, "these lines changed since") : null),
            h("p", { class: "said" }, c.text)
          );
        })
      ) : h("p", { class: "aside" }, "Nothing written yet. Select lines and :comment, or :note a symbol."),
      h("h2", null, "What ordo flagged"),
      serious.length ? h(
        "ol",
        { class: "flagged" },
        serious.map(
          ({ i, f }) => h(
            "li",
            null,
            h("div", { class: "where" }, h("span", { class: `dot ${f.level}` }), h("b", null, f.name), link(i, at2(r.items[i].path, f.row ?? r.items[i].new[0]))),
            h("p", { class: "said" }, prose(f.message.split("\n")[0]))
          )
        )
      ) : h("p", { class: "aside" }, "No verdicts or warnings."),
      minor,
      h("h2", null, "File by file"),
      h(
        "table",
        { class: "files" },
        h("thead", null, h("tr", null, h("th", null, "File"), h("th", null, "Read"), h("th", null, "Comments"), h("th", null, "Flagged"))),
        h(
          "tbody",
          null,
          fl.map((f) => {
            const next = f.hunks.find((i) => !r.items[i].reviewed) ?? f.hunks[0];
            return h(
              "tr",
              { class: f.read === f.hunks.length ? "done" : null },
              h("td", null, link(next, f.path)),
              h("td", null, h("span", { class: "meter" }, h("i", { style: `width:${100 * f.read / f.hunks.length}%` })), ` ${f.read} of ${f.hunks.length}`),
              h("td", null, f.comments || ""),
              h("td", null, LEVELS.filter((l) => l !== "note" && f.levels[l]).map((l) => h("span", { class: "lv" }, h("span", { class: `dot ${l}` }), f.levels[l])))
            );
          })
        )
      )
    )
  );
  page2.scrollTop = keep;
}
var HINTS = {
  list: [["next", "next hunk"], ["finding-next", "next finding"], ["command", "commands"]],
  code: [["next", "next line"], ["search", "search"], ["select-lines", "select"], ["jump-back", "back"]],
  why: [["next", "next link"], ["jump-to-edge", "go there"], ["jump-back", "back"]]
};
function renderHints() {
  const items = HINTS[st2.focus].map(([action, label]) => {
    const k = st2.keymap.keysFor(action)[0];
    return k ? h("span", null, h("kbd", null, k), label) : null;
  });
  replace($("#hints"), st2.notice ? h("span", { class: "notice" }, st2.notice) : items);
}
function zoomButton(p) {
  const on2 = st2.zoom && st2.focus === p;
  return h(
    "button",
    {
      class: "zoomer",
      title: on2 ? "Show all three columns" : "Give this column the whole page",
      "aria-label": on2 ? "restore the columns" : "maximise this column",
      onclick: () => {
        st2.zoom = !on2;
        st2.focus = p;
        renderAll();
      }
    },
    on2 ? "\u2921" : "\u2922"
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
  depmap = null;
  replace(ov);
  return true;
}
function showHelp() {
  const r = st2.review;
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
  document.body.classList.toggle("zoom", st2.zoom);
  for (const p of PANES) {
    const el = $(`#${p}`);
    el.classList.toggle("focus", st2.focus === p);
    el.classList.toggle("zoomed", st2.zoom && st2.focus === p);
  }
  renderBand();
  document.body.classList.toggle("summary", st2.summary && !!st2.review);
  $("#summary").hidden = !(st2.summary && st2.review);
  if (st2.summary && st2.review) return renderSummary();
  if (!st2.review) {
    for (const p of PANES) {
      replace($(`#${p} > .head`));
      replace($(`#${p} > .body`));
    }
    replace($("#code .foot .actions"));
    replace($("#code > .body"), h("div", { class: "loading" }, st2.empty ? "Nothing to read in this revision." : "Reading the change\u2026", h("small", null, st2.status)));
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
  const r = st2.review;
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
  if (paletteMode !== "command" || !st2.review) return [];
  const input = $("#palette-input").value;
  return complete(input, [...st2.review.commands, ...CLIENT_COMMANDS], argCandidates()).slice(0, 40);
}
function renderPalette() {
  const list = $("#palette-list");
  if (paletteMode !== "command" || !st2.review) return replace(list);
  const all = [...st2.review.commands, ...CLIENT_COMMANDS];
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
  if (name === "map") return openMap();
  if (name === "summary") return toggleSummary();
  if (name === "size") {
    const ui = sizeOf(arg);
    if (ui === null) return toast(`:size takes ${Object.keys(SIZES).join(", ")} or a factor like 1.2`, true);
    applySize(ui);
    localStorage.setItem("ordo.size", String(ui));
    return renderAll();
  }
  const out = await act({ op: "command", line_text: value });
  if (out && !out.error && name === "comment" && st2.from !== null) {
    st2.from = null;
    renderCode();
  }
}
function runSearch(query, forward = true) {
  if (!query) {
    st2.search = null;
    return renderCode();
  }
  const q2 = query.toLowerCase();
  const hits = st2.rows.flatMap((row, k2) => row.text.toLowerCase().includes(q2) ? [k2] : []);
  const after = hits.findIndex((x) => x > st2.cursor);
  const at2 = hits.indexOf(st2.cursor);
  const k = forward ? Math.max(0, at2 >= 0 ? at2 : after) : Math.max(0, (after < 0 ? hits.length : after) - (at2 >= 0 ? 2 : 1));
  st2.search = { query, hits, k };
  if (hits.length) st2.cursor = hits[k];
  st2.focus = "code";
  renderAll();
  followCursor();
}
function searchStep(by) {
  const s = st2.search;
  if (!s || !s.hits.length) return;
  s.k = (s.k + by + s.hits.length) % s.hits.length;
  st2.cursor = s.hits[s.k];
  renderCode();
  followCursor();
}
function page(by) {
  const body = $(`#${st2.focus} .body`);
  body.scrollTop += by * body.clientHeight;
}
function run(action) {
  const r = st2.review;
  if (!r) return;
  if (st2.summary && summaryKey(action)) return;
  const focus = st2.focus;
  switch (action) {
    case "quit":
      if (dismissSheet()) return;
      if (st2.search) st2.search = null;
      else if (st2.from !== null) st2.from = null;
      else if (st2.zoom) st2.zoom = false;
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
      const it2 = r.items[st2.sel];
      const name = it2?.symbols[0] ?? it2?.enclosing;
      const by = action === "symbol-next" ? 1 : -1;
      if (name && st2.search?.query === name) return searchStep(by);
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
      if (focus === "code") return moveCursor(-st2.rows.length);
      $("#why .body").scrollTop = 0;
      return;
    case "last":
      if (focus === "list") return void (stops().length && select(stops().at(-1)));
      if (focus === "code") return moveCursor(st2.rows.length);
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
      const at2 = s.indexOf(st2.sel);
      act({ op: "mark" }).then((out) => {
        if (out && !out.error && at2 >= 0 && at2 + 1 < s.length) select(s[at2 + 1]);
      });
      return;
    }
    case "focus-next":
    case "focus-prev": {
      const k = PANES.indexOf(focus) + (action === "focus-next" ? 1 : PANES.length - 1);
      st2.focus = PANES[k % PANES.length];
      return renderAll();
    }
    case "focus-list":
    case "focus-code":
    case "focus-why": {
      const p = action.slice(6);
      if (st2.focus === p) st2.zoom = !st2.zoom;
      st2.focus = p;
      return renderAll();
    }
    case "zoom":
      st2.zoom = !st2.zoom;
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
      if (focus === "why" && st2.whyLinks[st2.whyCur] !== void 0) jumpTo(st2.whyLinks[st2.whyCur]);
      return;
    case "jump-back":
      return jumpBack();
    case "mark-next":
    case "mark-prev": {
      const uses = new Set(r.items[st2.sel].uses_at.flatMap((u) => u.rows));
      return cursorTo((row) => row.kind !== "del" && uses.has(row.n), action === "mark-next");
    }
    case "comment-next":
    case "comment-prev": {
      const c = new Set(st2.file?.comments.map((c2) => c2.start) ?? []);
      return cursorTo((row) => row.kind !== "del" && c.has(row.n), action === "comment-next");
    }
    case "para-next":
    case "para-prev":
      return cursorTo((row) => row.text.trim() === "", action === "para-next");
    case "cursor-left":
    case "cursor-right":
    case "line-start":
    case "line-end": {
      const text = st2.rows[st2.cursor]?.text ?? "";
      const last = Math.max(0, [...text].length - 1);
      const now = st2.col === null ? firstWordCol(text) : caretCol(text);
      st2.col = action === "line-start" ? 0 : action === "line-end" ? last : Math.max(0, Math.min(last, now + (action === "cursor-right" ? 1 : -1)));
      st2.focus = "code";
      return renderAll();
    }
    case "word-next":
    case "word-prev":
    case "word-end": {
      const kind = action === "word-next" ? "next" : action === "word-prev" ? "prev" : "end";
      const text = st2.rows[st2.cursor]?.text ?? "";
      const to = wordMove(
        st2.rows.map((r2) => r2.text),
        { row: st2.cursor, col: st2.col === null ? firstWordCol(text) : caretCol(text) },
        kind
      );
      st2.col = to.col;
      st2.focus = "code";
      if (to.row === st2.cursor) return renderAll();
      return cursorToRow(to.row);
    }
    case "select-lines":
      st2.from = st2.from === null ? st2.cursor : null;
      st2.focus = "code";
      return renderAll();
    case "comment":
      return openPalette("command", "comment ");
    case "reload":
      act({ op: "reload" });
      return;
    case "hover":
      if (focus === "why" && st2.whyLinks[st2.whyCur] !== void 0) return void previewHunk(st2.whyLinks[st2.whyCur]);
      act({ op: "hover", col: st2.col === null ? firstWordCol(st2.rows[st2.cursor]?.text ?? "") : caretCol(st2.rows[st2.cursor]?.text ?? "") });
      return;
    case "open-editor":
      return openEditor();
    case "open-deps":
    case "focus-deps":
      return void openCanvas();
  }
}
function whyStep(by) {
  const n = st2.whyLinks.length;
  if (!n) {
    $("#why .body").scrollTop += by * 40;
    return;
  }
  st2.whyCur = Math.max(0, Math.min(n - 1, st2.whyCur + by));
  renderWhy();
  $("#why tr.cur")?.scrollIntoView({ block: "nearest" });
}
function onKey(e) {
  if (!$("#peek").hidden) hidePeek();
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
  if (key === "Enter" && depmap) {
    e.preventDefault();
    return void mapKey("jump-to-edge");
  }
  if (key === "Esc" && $("#overlay").dataset.kind === "config") {
    e.preventDefault();
    return void dismissSheet();
  }
  if (key === "Enter" && st2.focus === "why" && st2.whyCur >= 0) {
    e.preventDefault();
    return run("jump-to-edge");
  }
  const pending = st2.pending;
  const res = st2.keymap.resolve(pending, key);
  clearTimeout(st2.pendingTimer);
  st2.pending = null;
  if ("pending" in res) {
    e.preventDefault();
    st2.pending = key;
    const bare = st2.keymap.bare(key);
    if (bare) st2.pendingTimer = window.setTimeout(() => (st2.pending = null, run(bare)), CHORD_TIMEOUT);
    return;
  }
  if ("act" in res) {
    e.preventDefault();
    if (canvasKey(res.act) || mapKey(res.act)) return;
    return run(res.act);
  }
  if (pending) {
    const bare = st2.keymap.bare(pending);
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
  const t = st2.review.items[i];
  return `${t.symbols[0] ?? t.enclosing ?? t.symbol ?? basename(t.path)} \xB7 ${basename(t.path)}:${t.new[0]}`;
}
async function previewHunk(i) {
  const t = st2.review.items[i];
  const f = await fileOf(t.path);
  showSheet(hunkTitle(i), h("div", null, h("p", { class: "muted" }, t.rationale), h("div", { class: "src" }, excerpt(f, i, 2, 40))));
}
var canvas = null;
async function openCanvas() {
  const r = st2.review;
  const it2 = r.items[st2.sel];
  const cards = it2.edges.filter((e) => e.target !== null && r.items[e.target]).map((e) => ({ i: e.target, needs: e.needs }));
  if (!cards.length) return toast("nothing in this change needs this hunk, and it needs nothing");
  canvas = { anchor: st2.sel, cards, sel: 0 };
  await renderCanvas();
}
async function renderCanvas() {
  if (!canvas) return;
  const c = canvas;
  const files2 = await Promise.all([c.anchor, ...c.cards.map((k) => k.i)].map((i) => fileOf(st2.review.items[i].path)));
  if (canvas !== c) return;
  const card = (k) => {
    const { i } = c.cards[k];
    return h(
      "div",
      { class: `card${k === c.sel ? " cur" : ""}`, onclick: () => goCanvas(k) },
      h("header", null, hunkTitle(i), h("span", { class: "muted" }, st2.review.items[i].rationale)),
      excerpt(files2[k + 1], i, 1, 14)
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
      h("div", { class: "anchor" }, excerpt(files2[0], c.anchor, 1, 16)),
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
  } else if (action === "open-deps" || action === "focus-deps") {
    const root = canvas.anchor;
    canvas = null;
    depmap = { root, sel: root, order: [] };
    renderMap();
  } else if (action === "quit") {
    canvas = null;
    closeSheet();
  }
  return true;
}
var depmap = null;
function openMap() {
  const r = st2.review;
  if (!needPairs(r.items, r.view).some((p) => p.includes(st2.sel))) {
    return toast("nothing in this change needs this hunk, and it needs nothing");
  }
  depmap = { root: st2.sel, sel: st2.sel, order: [] };
  renderMap();
}
function renderMap(again = false) {
  if (!depmap) return;
  const r = st2.review;
  const root = depmap.root;
  if (!r.view.includes(root)) {
    closeSheet();
    return;
  }
  const scrolled = $("#overlay").querySelector(".sheet > .scroll");
  const keep = again && scrolled ? { top: scrolled.scrollTop, left: scrolled.scrollLeft } : null;
  const ui = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--ui")) || 1;
  const box = { width: 236 * ui, height: 54 * ui };
  const m = depMap(r.items, r.view, root, box);
  depmap.order = r.view.filter((i) => m.nodes.has(i));
  if (!m.nodes.has(depmap.sel)) depmap.sel = root;
  const pick = (i) => {
    if (!depmap || depmap.sel === i) return;
    depmap.sel = i;
    paintMap();
  };
  const node = (i, at2) => {
    const it2 = r.items[i];
    const name = it2.symbols[0] ?? it2.enclosing ?? it2.symbol ?? basename(it2.path);
    return h(
      "button",
      {
        class: `node${i === root ? " root" : ""}`,
        dataset: { i: String(i) },
        style: `left:${at2.x - box.width / 2}px;top:${at2.y - box.height / 2}px;width:${box.width}px;height:${box.height}px`,
        title: it2.rationale,
        onmouseenter: () => pick(i),
        onfocus: () => pick(i),
        onclick: () => goMap(i)
      },
      h("span", { class: "name" }, it2.reviewed ? `\u2713 ${name}` : name),
      h("span", { class: "at" }, `${basename(it2.path)}:${it2.new[0]}`),
      it2.level && h("span", { class: `dot ${it2.level}`, title: `${it2.level} finding` })
    );
  };
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("width", String(m.width));
  svg.setAttribute("height", String(m.height));
  svg.setAttribute("aria-hidden", "true");
  svg.innerHTML = '<defs><marker id="map-head" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0 L8 4 L0 8 z" fill="context-stroke"/></marker></defs>';
  for (const e of m.edges) {
    const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
    p.setAttribute("d", smoothPath(e.points));
    p.setAttribute("marker-end", "url(#map-head)");
    p.dataset.from = String(e.from);
    p.dataset.to = String(e.to);
    if (e.outOfOrder) p.classList.add("early");
    svg.append(p);
  }
  const ground = groundOf(r.items, r.view, root).size;
  const strands = cascadeOf(r.items, r.view, root).size;
  const hunks = (n) => `${n} hunk${n === 1 ? "" : "s"}`;
  const early = m.edges.filter((e) => e.outOfOrder).length;
  showSheet(
    `Dependencies of ${hunkTitle(root)}`,
    h(
      "div",
      { class: "mapview" },
      h(
        "p",
        { class: "lede" },
        `It stands on ${hunks(ground)}, drawn to its left, and ${hunks(strands)} ${strands === 1 ? "stands" : "stand"} on it, drawn to its right.`,
        early ? h("span", { class: "early" }, ` ${hunks(early)} marked reviewed before what ${early === 1 ? "it needs" : "they need"}, dashed.`) : null
      ),
      h("p", { class: "said", "aria-live": "polite" }),
      h("div", { class: "depmap", style: `width:${m.width}px;height:${m.height}px` }, svg, [...m.nodes].map(([i, at2]) => node(i, at2)))
    )
  );
  $("#overlay").querySelector(".sheet")?.classList.add("wide");
  $("#overlay").dataset.kind = "map";
  paintMap();
  const scroll = $("#overlay").querySelector(".sheet > .scroll");
  if (keep && scroll) {
    scroll.scrollTop = keep.top;
    scroll.scrollLeft = keep.left;
  } else {
    $("#overlay .node.root")?.scrollIntoView({ block: "center", inline: "center" });
  }
}
function paintMap() {
  if (!depmap) return;
  const r = st2.review;
  const i = depmap.sel;
  const strands = cascadeOf(r.items, r.view, i);
  const needs = new Set(needPairs(r.items, r.view).flatMap(([from, to]) => to === i ? [from] : []));
  for (const el of $("#overlay").querySelectorAll(".node")) {
    const k = Number(el.dataset.i);
    el.classList.toggle("picked", k === i);
    el.classList.toggle("strands", strands.has(k));
    el.classList.toggle("needed", needs.has(k));
    el.classList.toggle("reviewed", r.items[k].reviewed);
  }
  for (const p of $("#overlay").querySelectorAll(".depmap path[data-from]")) {
    const from = Number(p.dataset.from);
    const to = Number(p.dataset.to);
    p.classList.toggle("strands", (from === i || strands.has(from)) && strands.has(to));
    p.classList.toggle("needed", to === i);
  }
  const said = $("#overlay").querySelector(".mapview .said");
  if (said && r.items[i]) {
    const name = hunkTitle(i);
    const stranded = strands.size ? `rejecting it strands ${strands.size} hunk${strands.size === 1 ? "" : "s"}, shaded red` : "nothing here needs it";
    const needed = needs.size ? `; it needs ${needs.size}, outlined` : "";
    said.textContent = `${i === depmap.root ? "This hunk" : name}: ${stranded}${needed}. Click a hunk to read it; gD on it draws its own chain.`;
  }
}
function goMap(i) {
  depmap = null;
  closeSheet();
  jumpTo(i);
}
function mapKey(action) {
  if (!depmap || action === "command" || action === "command-goto") return false;
  const order = depmap.order;
  if (action === "next" || action === "prev") {
    const at2 = order.indexOf(depmap.sel);
    depmap.sel = order[Math.max(0, Math.min(order.length - 1, at2 + (action === "next" ? 1 : -1)))];
    paintMap();
    $("#overlay .node.picked")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  } else if (action === "jump-to-edge") {
    goMap(depmap.sel);
  } else if ((action === "open-deps" || action === "focus-deps") && depmap.sel !== depmap.root) {
    depmap.root = depmap.sel;
    renderMap();
  } else if (action === "quit" || action === "open-deps" || action === "focus-deps") {
    depmap = null;
    closeSheet();
  }
  return true;
}
function renderConfig() {
  const cfg = st2.review?.config;
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
        if (st2.review) renderList();
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
      if (st2.focus !== p) {
        st2.focus = p;
        renderHints();
        for (const q2 of PANES) $(`#${q2}`).classList.toggle("focus", q2 === p);
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
  window.addEventListener("resize", () => {
    if (!st2.review) return;
    renderList();
    renderStrip();
  });
  $("#peek").addEventListener("mouseleave", () => hidePeek());
  wireSplits();
  api.events((event, data) => {
    if (event === "progress" && !st2.review) {
      st2.status = data.status;
      renderAll();
    } else if (event === "progress") {
      st2.notice = data.status;
      renderHints();
    } else if (event === "changed" && data.version !== st2.version) {
      st2.notice = "";
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
