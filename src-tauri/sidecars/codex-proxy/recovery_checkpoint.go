package main

func shouldCommitRecoveryCheck(attempted bool, succeeded bool) bool {
	return !attempted || succeeded
}
