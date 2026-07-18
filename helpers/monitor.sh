#!/bin/bash
export NEWEST_ATTACK="`ls -r results/attack*.txt | head -n1`"
echo "Current File: $NEWEST_ATTACK"
printf "Broken: "; rg "broken" $NEWEST_ATTACK | wc -l
printf "Successes: "; rg "success" $NEWEST_ATTACK | wc -l
echo "###########################################"
tail -n 15 $NEWEST_ATTACK