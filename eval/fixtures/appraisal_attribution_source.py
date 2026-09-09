"""Independent oracle for the registered context-attribution cases."""
import json
from pathlib import Path
import appraisal_mismatch_source as source
source.CASES=json.loads((Path(__file__).resolve().parent/'appraisal-attribution/cases.json').read_text())
if __name__=='__main__':source.main()
