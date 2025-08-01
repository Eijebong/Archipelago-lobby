import "./App.css";
import { Suspense } from "react";
import DiffViewer from "./components/DiffViewer";

function App() {
    const taskId = window.location.pathname;

    return (
        <Suspense fallback={<div>Loading...</div>}>
            <DiffViewer taskId={taskId} />
        </Suspense>
    );
}

export default App;
