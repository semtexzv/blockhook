#!/usr/bin/env bash

export DATABASE_URL=$(mktemp)

diesel migration run
diesel print-schema