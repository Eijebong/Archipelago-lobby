import { DiffFile, DiffModeEnum, DiffView } from "@git-diff-view/react";
import "@git-diff-view/react/styles/diff-view.css";
import "./file-diff.css";
import DiffWorker from "worker-loader!./diff.worker.ts";
import { useEffect, useRef, useState } from "react";

const w = new DiffWorker();

w.onerror = (e) => {
    console.error("WORKER ERROR:", e.message, "at", e.filename, ":", e.lineno);
};

function parseGitDiff(diff: string) {
    const fileDiffs = diff.split(/(?=diff --git)/); // Split at each file boundary
    const result: any[] = [];

    fileDiffs.forEach((fileDiff) => {
        const lines = fileDiff.split("\n");
        let filenameBefore = "";
        let filenameAfter = "";
        let isBinary = false;
        var hunks = "";

        if (lines[0].startsWith("diff --git")) {
            const regex = /^diff --git a\/(.+?) b\/(.+?)$/;
            const match = lines[0].match(regex)
            filenameBefore = match[1]
            filenameAfter = match[2]
        }

        lines.forEach((line) => {
            if (line.startsWith("+++")) {
                filenameAfter = line.slice(4)
                if (filenameAfter.startsWith('b/')) {
                    filenameAfter = filenameAfter.slice(2)
                }
            }
            if (line.startsWith("---")) {
                filenameBefore = line.slice(4)
                if (filenameBefore.startsWith('a/')) {
                    filenameBefore = filenameBefore.slice(2)
                }
            }
            if (line.startsWith("deleted file mode")) {
                filenameAfter = "/dev/null"
            }
            hunks += `${line.replace("\r", "")}\n`;
            if(line.startsWith("Binary files")) {
                isBinary = true
            }
        });

        hunks = hunks.slice(0, -1);

        if (filenameBefore || filenameAfter) {
            const file = { filenameBefore, filenameAfter, hunks, isBinary };
            result.push(file);
        }
    });

    return result;
}

function FileDiffViewHeader({ diff_content }: { diff_content: any }) {
    const isAddition = diff_content.filenameBefore === "/dev/null";
    const isRemoval = diff_content.filenameAfter === "/dev/null";

    const displayFileName = !isRemoval
        ? diff_content.filenameAfter
        : diff_content.filenameBefore;

    var action = "changed";
    if (isAddition) {
        action = "added";
    } else if (isRemoval) {
        action = "removed";
    }

    return (
        <div className="file-header">
            <span>{displayFileName}</span>
            <span className={`action-tag ${action}-tag`}>{action}</span>
        </div>
    );
}

const fileMap = new Map<string, DiffFile>();
function FileDiffView({
    diff_content,
    annotations,
}: {
    diff_content: any;
    annotations: any;
}) {
    const [diffFile, setDiffFile] = useState(
        fileMap.get(diff_content.filenameAfter),
    );
    const [isLoading, setLoading] = useState(false);
    const thisId = useRef(null);

    useEffect(() => {
        setLoading(true);
        thisId.current = Math.random();
        w.postMessage({ id: thisId.current, data: diff_content });

        const cb = (event: MessageEvent<any>) => {
            if (event.data.id === thisId.current) {
                const d = DiffFile.createInstance(
                    event.data.data,
                    event.data.bundle,
                );
                setDiffFile(d);
                fileMap.set(event.data.data.filenameAfter, d);
                setLoading(false);
            }
        };
        w.addEventListener("message", cb);

        return () => w.removeEventListener("message", cb);
    }, [diff_content]);

    const renderExtendLine = ({ data }: any) => {
        return (
            <div style={{ backgroundColor: "#610505", color: "white" }}>
                {data.desc}
            </div>
        );
    };
    const fileAnnotations = annotations[diff_content.filenameAfter];

    return (
        <div>
            <FileDiffViewHeader diff_content={diff_content} />
            {!isLoading ? (
                diff_content.isBinary ? <div style={{backgroundColor: "#610505", "color": "white"}}>Binary file</div> :
                <DiffView
                    extendData={{ newFile: fileAnnotations }}
                    renderExtendLine={renderExtendLine}
                    diffFile={diffFile}
                    diffViewTheme="dark"
                    diffViewHighlight={true}
                    diffViewMode={DiffModeEnum.Unified}
                    style={{
                        marginBottom: "1em"
                    }}
                />
            ) : (
                <span>Loading file...</span>
            )}
        </div>
    );
}

function DiffVersionElement({ diff, annotations }: any) {
    var diff_content = null;

    if (diff[1] !== "VersionRemoved") {
        diff_content = parseGitDiff(diff[1].VersionAdded);
    }

    const [_, nextVersion] = diff[0].split("...");

    const versionAnnotations: Record<string, any[]> = annotations[nextVersion];
    const transformed: Record<string, any> = {};

    if (versionAnnotations) {
        for (const [fileName, issues] of Object.entries(versionAnnotations)) {
            transformed[fileName] = {};
            for (const issue of issues) {
                const lineNumber = issue.line;
                transformed[fileName][lineNumber] = { data: { ...issue } };
            }
        }
    }

    if (!nextVersion) return null;

    return (
        <div id={diff[0]}>
            <VersionElement version={diff[0]} noRemoval={true} />

            {diff_content.map((diff_file, index) => (
                <FileDiffView
                    key={index}
                    annotations={transformed}
                    diff_content={diff_file}
                />
            ))}
        </div>
    );
}

function VersionElement({
    version,
    noRemoval,
}: {
    version: string;
    noRemoval: boolean;
}) {
    const [oldVersion, newVersion] = version.split("...");

    if (!oldVersion && newVersion) {
        return <a href="{`#${newVersion}`}">✅ {newVersion} </a>;
    }

    if (oldVersion && !newVersion) {
        if (noRemoval) return null;
        return <span>❌ {oldVersion}</span>;
    }

    return (
        <a href={`#${newVersion}`}>
            {oldVersion} → {newVersion}
        </a>
    );
}

function DiffElement({ apworld_diff }: any) {
    const [diffs, annotations] = apworld_diff;
    return (
        <>
            <ul>
                {Object.entries(diffs.diffs).map((diff: any) => (
                    <li key={diff[0]}>
                        <VersionElement
                            version={diff[0]}
                            noRemoval={false}
                        />
                    </li>
                ))}
            </ul>

            <hr />

            {Object.entries(diffs.diffs).map((diff: any) => (
                <DiffVersionElement
                    diff={diff}
                    annotations={annotations}
                    key={diff[0]}
                />
            ))}
        </>
    );
}

export default DiffElement;
