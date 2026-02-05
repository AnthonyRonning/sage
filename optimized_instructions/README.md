# Optimized Instructions

This directory stores GEPA-optimized agent instructions.

## Files

- `baseline.txt` - The original manually-engineered AGENT_INSTRUCTION (copied from sage_agent.rs)
- `gepa_dev.txt` - Development mode optimization result
- `gepa_production.txt` - Production mode optimization result
- `evolution_history.json` - History of optimization runs

## Usage

1. Run optimization: `just gepa-optimize-dev`
2. Compare results: `just gepa-compare`
3. If optimized is better, update `AGENT_INSTRUCTION` in `sage_agent.rs`

## Notes

- All files in this directory are gitignored (except README)
- The baseline is preserved in git history via sage_agent.rs
- Each optimization run creates a timestamped backup
