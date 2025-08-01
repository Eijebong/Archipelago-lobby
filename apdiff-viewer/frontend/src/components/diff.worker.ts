import { DiffFile } from "@git-diff-view/react";
import { getDiffViewHighlighter } from "./hl/index";

const ctx: Worker = self as any;

ctx.addEventListener("message", async (event) => {
    const data = event.data.data;

    const hl = await getDiffViewHighlighter();
    hl.setMaxLineToIgnoreSyntax(100000);

    const file = new DiffFile(
        data.filenameBefore === "/dev/null" ? null : data.filenameBefore,
        null,
        data.filenameAfter === "/dev/null" ? null : data.filenameAfter,
        null,
        [data.hunks],
        null,
        null,
    );
    file.initTheme("dark");
    file.initRaw();
    file.initSyntax({ registerHighlighter: hl });
    file.buildSplitDiffLines();
    file.buildUnifiedDiffLines();
    const bundle = file._getFullBundle();
    ctx.postMessage({
        id: event.data.id,
        data: event.data.data,
        bundle: bundle,
    });
    file.clear();
});
