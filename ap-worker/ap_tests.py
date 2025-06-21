import sys
import os

if len(sys.argv) != 8:
    print("Usage: ap_test.py worlds_dir custom_worlds_dir apworld_name world_version world_name annotations_folder output_folder")
    sys.exit(1)

ap_path = os.path.abspath(os.path.dirname(sys.argv[0]))
sys.path.insert(0, ap_path)

import copy
import handler
import itertools
import json
import tomlkit
import unittest
import re
from collections import defaultdict
from test.bases import WorldTestBase
from Fill import FillError
from worlds.AutoWorld import AutoWorldRegister
from worlds.Files import AutoPatchRegister
import test.general.test_fill
import test.general.test_ids
import warnings
from semver import Version
warnings.simplefilter("ignore")

class Annotation():
    def __init__(self, params):
        self._version_min = self.get_min_version_bound(params.get('__version__'))
        self._version_max = self.get_max_version_bound(params.get('__version__'))
        self._params = params

    def get_expected_result(self):
        return self._params["__expected__"].lower()

    def get_max_version_bound(self, version):
        if version is None:
            return None

        if '<' not in version:
            return None

        *rest, max_version = version.split('<', 1)
        return Version.parse(max_version)

    def get_min_version_bound(self, version):
        if version is None:
            return None

        if version.startswith(">="):
            min_version, *rest = version[len(">="):].split(',', 1)
            return Version.parse(min_version)
        elif version.startswith('<'):
            return None

        return Version.parse(version)

    def is_valid_for_version(self, version):
        if version is None:
            return True

        # No version requirement
        if self._version_min is None and self._version_max is None:
            return True

        version = Version.parse(version)

        if self._version_min is None:
            return version < self._version_max

        if self._version_max is None:
            return version >= self._version_min

        return self._version_min <= version < self._version_max

    def to_toml(self):
        t = { k: v for k, v in self._params.items() }

        if self._version_min is not None or self._version_max is not None:
            if self._version_min is None:
                version = f"<{self._version_max}"
            elif self._version_max is None:
                version = f">={self._version_min}"
            else:
                version = f">={self._version_min},<{self._version_max}"
            t["__version__"] = version
        return t


def get_annotations_for_game(annotations_folder, apworld_name, version):
    annotations = defaultdict(lambda: [])

    annotation_path = os.path.join(annotations_folder, f"{apworld_name}.toml")

    # If there's no annotations, try out the same path but replacing spaces in the apworld name with a +
    # In theory this shouldn't be necessary but unfortunately there exists some apworlds with spaces in their name (namely twilight princess)
    # and taskcluster is replacing spaces in artifact names with pluses for some reason. If that ever changes then this codepath will become
    # useless, but until then, just try out...
    if not os.path.isfile(annotation_path):
        annotation_path = os.path.join(annotations_folder, "{}.toml".format(apworld_name.replace(" ", "+")))

    if not os.path.isfile(annotation_path):
        return annotations

    with open(annotation_path, "rb") as fd:
        raw_annotations = tomlkit.load(fd)

    for name, params in raw_annotations.items():
        for param in params:
            annotation = Annotation(param)
            if annotation.is_valid_for_version(version):
                annotations[name].append(annotation)

    return annotations


def _get_new_expectations_from(annotations_folder, apworld_name, results, current_version):
    unexpected_results = itertools.chain(
        ((test, "fail") for (test, _) in results.failures),
        ((test, "error") for (test, _) in results.errors),
        ((test, "success") for test in results.unexpectedSuccesses)
    )

    new_annotations = get_annotations_for_game(annotations_folder, apworld_name, None)

    for (test, unexpected_result) in unexpected_results:
        test_id = _test_id(test)
        current_annotation = _get_annotation(test, new_annotations, version=current_version)

        # If there's currently no annotation for this failure, add one, with version >=current_version
        if current_annotation is None:
            # Don't write expectations for unexpected success
            if unexpected_result == "success":
                continue

            params = {}
            params["__expected__"] = unexpected_result

            new_annotation = Annotation(params)
            new_annotations[test_id].append(new_annotation)
            new_annotation._version_min = Version.parse(current_version)
        else:
            # If there's one, and it's different than the current, mark it as being valid up to this version
            if current_annotation.get_expected_result() != unexpected_result:
                current_annotation._version_max = Version.parse(current_version)

            # Then create a new annotation for this version onwards with the new expectation, only if the new expectation isn't success
            if unexpected_result == "success":
                continue

            params = {}
            params["__expected__"] = unexpected_result
            new_annotation = Annotation(params)
            new_annotations[test_id].append(new_annotation)
            new_annotation._version_min = Version.parse(current_version)


    serialized = {name: [annotation.to_toml() for annotation in annotations] for name, annotations in new_annotations.items()}
    return tomlkit.dumps(serialized)



def _test_id(test):
    # We don't care about the seed when looking at expectations
    return re.sub("seed=[0-9]+", "seed=<random>", test.id())


def _get_annotation(test, annotations, *, version=None, ignore_params=False):
    test_id = _test_id(test)

    if test_id in annotations:
        test_annotations = annotations[test_id]
        for annotation in test_annotations:
            if version is not None and not annotation.is_valid_for_version(version):
                continue

            return annotation
    return None


def _expected_result(test, annotations, *, ignore_params=False):
    annotation = _get_annotation(test, annotations, ignore_params=ignore_params)
    if annotation is None:
        return "success"

    return annotation.get_expected_result()


if __name__ == "__main__":
    apworlds_dir = sys.argv[1]
    custom_apworlds_dir = sys.argv[2]
    apworld = sys.argv[3]
    version = sys.argv[4]
    world_name = sys.argv[5]
    annotations_folder = sys.argv[6]
    output_folder = sys.argv[7]

    os.makedirs(output_folder, exist_ok=True)
    ap_handler = handler.ApHandler(apworlds_dir, custom_apworlds_dir)
    ap_handler.load_apworld(apworld, version)

    # Unload as many worlds as possible before running tests
    loaded_worlds = list(AutoWorldRegister.world_types.keys())
    for loaded_world in loaded_worlds:
        # Those 2 worlds are essential to testing (who could've seen this coming, yet another dependency on ALTTP)
        if loaded_world in ("Test Game", "A Link to the Past"):
            continue

        if loaded_world != world_name:
            del AutoWorldRegister.world_types[loaded_world]

            if loaded_world in AutoPatchRegister.patch_types:
                del AutoPatchRegister.patch_types[loaded_world]

    annotations = get_annotations_for_game(annotations_folder, apworld, version)

    class WorldTest(WorldTestBase):
        game = world_name

    class MyResult(unittest.TextTestResult):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            self.has_fill_errors = False


        def _shouldIgnoreResult(self, test):
            if _expected_result(test, annotations) == "flaky":
                return True

            # If the current test is a subtest
            if hasattr(test, "test_case"):
                expected = _expected_result(test.test_case, annotations, ignore_params=True)
                return expected == "error"
            return False


        def addFailure(self, test, err):
            if self._shouldIgnoreResult(test):
                self.addSkip(test, "A subtest is failing for a test that errors out")
                return

            expected  = _expected_result(test, annotations)
            if expected == "fail":
                self.addExpectedFailure(test, err)
            else:
                super().addFailure(test, err)

        def addError(self, test, err):
            if self._shouldIgnoreResult(test):
                self.addSkip(test, "A subtest is failing for a test that errors out")
                return

            if isinstance(err[1], FillError):
                self.has_fill_errors = True

            expected  = _expected_result(test, annotations)
            if expected == "error":
                super().addExpectedFailure(test, err)
            else:
                super().addError(test, err)

            self.stop()

        def addSuccess(self, test):
            if self._shouldIgnoreResult(test):
                self.addSkip(test, "A subtest is failing for a test that errors out")
                return

            expected = _expected_result(test, annotations)
            if expected == "success":
                super().addSuccess(test)
            else:
                super().addUnexpectedSuccess(test)

        def addSubTest(self, test, subtest, err):
            if err is None:
                return self.addSuccess(subtest)

            if issubclass(err[0], test.failureException):
                self.addFailure(subtest, err)
            else:
                self.addError(subtest, err)

        def hasFillErrors(self):
            return self.has_fill_errors

    runner = unittest.TextTestRunner(verbosity=1, resultclass=MyResult)

    suite = unittest.TestSuite()
    suite.addTests(unittest.defaultTestLoader.loadTestsFromTestCase(WorldTest))
    suite.addTests(unittest.defaultTestLoader.discover("test/general", top_level_dir="."))
    results = runner.run(suite)

    if not results.wasSuccessful or results.expectedFailures:
        output = {
            "failures": {fail.id(): {"traceback": tb, "description": fail.shortDescription()} for fail, tb in results.failures},
            "errors": {error.id(): {"traceback": tb, "description": error.shortDescription()} for error, tb in results.errors},
            "expected_failures": {fail.id(): {"traceback": tb, "description": fail.shortDescription()} for fail, tb in results.expectedFailures},
            "unexpected_successes": {success.id(): {"description": success.shortDescription()} for success in results.unexpectedSuccesses},
            "apworld": apworld,
            "version": version,
            "world_name": world_name
        }

        with open(os.path.join(output_folder, f"{apworld}.aptest"), "w") as fd:
            fd.write(json.dumps(output))

    new_expectations = _get_new_expectations_from(annotations_folder, apworld, results, version)
    with open(os.path.join(output_folder, "{}.toml".format(apworld)), "w") as fd:
        fd.write(new_expectations)

    if not results.wasSuccessful():
        if results.hasFillErrors():
            sys.exit(69)
        sys.exit(1)

    print(f"Successfully validated {apworld} {version}")
