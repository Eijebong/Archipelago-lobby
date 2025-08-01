import { use, useEffect, useState } from "react";
import DiffElement from "./DiffElement";
import "./diff-viewer.css";

async function fetchDiffs(taskId: string) {
    const response = await fetch(`/api/diffs/${taskId}`);
    return response.json();
}

function DiffViewer({ taskId }: { taskId: string }) {
    const [diffPromise, setDiffPromise] = useState<Promise<any> | null>(null);
    const [currentDiff, setCurrentDiff] = useState(null);

    useEffect(() => {
        const p = fetchDiffs(taskId);
        setDiffPromise(p);
    }, [taskId]);

    if (diffPromise === null) {
        return null;
    }

    const apworlds_diffs = use(diffPromise);

    return (
        <>
            <nav>
                {apworlds_diffs.map((apworld_diff: any, index: number) => (
                    <a
                        href="#"
                        key={index}
                        className={
                            currentDiff == null || currentDiff === apworld_diff
                                ? "selected"
                                : ""
                        }
                        onClick={() => setCurrentDiff(apworld_diff)}
                    >
                        {apworld_diff[0].world_name || "Unknown"}
                    </a>
                ))}
            </nav>
            <div>
                <DiffElement apworld_diff={currentDiff || apworlds_diffs[0]} />
            </div>
        </>
    );
}

export default DiffViewer;
