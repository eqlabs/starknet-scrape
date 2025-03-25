use alloy::sol;

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface StarknetCore {
        event LogStateUpdate(uint256 globalRoot, int256 blockNumber, uint256 blockHash);
    }
);
