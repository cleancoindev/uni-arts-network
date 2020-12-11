#![cfg_attr(not(feature = "std"), no_std)]

use frame_system::{
	self as system,
	offchain::{
		AppCrypto, CreateSignedTransaction,
	}
};
use frame_support::{
	debug, decl_module, decl_storage, decl_event,
	traits::Get,
};
use sp_core::crypto::KeyTypeId;
use sp_runtime::{
	transaction_validity::{
		ValidTransaction, TransactionValidity, TransactionSource,
		TransactionPriority,
	},
	offchain::{http},
};
use sp_runtime::{traits::{Hash}};
use ethereum_types::{H64, H128, H160, U256, H256, H512};
use lite_json::json::JsonValue;

#[derive(Encode, Decode)]
pub struct RpcUrl {
	url: Vec<u8>,
}

///information about a erc20 transfer.
#[derive(Clone, Encode, Decode)]
pub struct TransferInfo {
	pub block_number: U256,
	pub time_stamp: U256,
	pub tx_hash: H256,
	pub nonce: u64,
	pub block_hash: H256,
	pub from_address: H160,
	pub to_address: H160,
	pub contract_address: H160,
	pub value: U256,
	pub token_name: Vec<u8>,
	pub token_symbol: Vec<u8>,
	pub token_decimal: u64,
	pub transaction_index: u64,
	pub gas: U256,
	pub gas_price: U256,
	pub gas_used: U256,
	pub cumulative_gas_used: U256,
	pub confirmations: U256,
}

/// This pallet's configuration trait
pub trait Trait: CreateSignedTransaction<Call<Self>> {
	/// The identifier type for an offchain worker.
	type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

	/// The overarching event type.
	type Event: From<Event<Self>> + Into<<Self as frame_system::Trait>::Event>;
	/// The overarching dispatch call type.
	type Call: From<Call<Self>>;

	// Configuration parameters

	/// A grace period after we send transaction.
	///
	/// To avoid sending too many transactions, we only attempt to send one
	/// every `GRACE_PERIOD` blocks. We use Local Storage to coordinate
	/// sending between distinct runs of this offchain worker.
	type GracePeriod: Get<Self::BlockNumber>;

	/// Number of blocks of cooldown after unsigned transaction is included.
	///
	/// This ensures that we only accept unsigned transactions once, every `UnsignedInterval` blocks.
	type UnsignedInterval: Get<Self::BlockNumber>;

	/// A configuration for base priority of unsigned transactions.
	///
	/// This is exposed so that it can be tuned for particular runtime, when
	/// multiple pallets send unsigned transactions.
	type UnsignedPriority: Get<TransactionPriority>;
}

decl_storage! {
	trait Store for Module<T: Trait> as EtherscanWorkerModule {
		/// Current synchronization block height.
		pub SyncBlockNumber get(fn sync_block_number): Option<U256>;

		/// Ethereum Erc20 Token Name
		pub Erc20TokenName get(fn erc20_token_name): Option<Vec<u8>>;

		/// Ethereum Erc20 Token Address
		pub Erc20TokenAddress get(fn erc20_token_address): Option<H160>;

		/// Mapping Token hash
		pub MappingTokenHash get(fn mapping_token_hash): Option<Hash>;

		/// Start synchronization block height
		pub SyncBeginBlockHeight get(fn sync_begin_block_heigh): Option<U256>;

		/// Sync block height
		pub SyncBlockHeight get(fn sync_block_heigh): Option<U256>;

		/// Current block height
		pub CurrentBlockHeight get(fn current_block_heigh): Option<U256>;

		/// We store block erc20 transfer tx hash
		pub BlockTransfers get(fn block_number_transfers): map hasher(blake2_128_concat) U256 => H160;

		/// We store full information about the erc20 transfer
		pub TxHashTransferList get(fn transfer_id): double_map hasher(blake2_128_concat) H256, hasher(blake2_128_concat) u64,  => TransferInfo;

		/// All erc20 transfer information in a block
		pub BlockTransferList get(fn all_transfer): hasher(blake2_128_concat) U256, hasher(blake2_128_concat) u64 => TransferInfo;

		/// RpcUrls set by anyone
		pub RpcUrls get(fn rpc_urls): map hasher(twox_64_concat) T::AccountId => Option<RpcUrl>;

	}
}

decl_event!(
	pub enum Event<T> where AccountId = <T as frame_system::Trait>::AccountId {
		NewHeader(u32, AccountId),
	}
);

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		// // Errors must be initialized if they are used by the pallet.
		// type Error = Error<T>;

		// Events must be initialized if they are used by the pallet.
		fn deposit_event() = default;

		#[weight = 0]
		fn init(
			origin,
			erc20_token_name: Vec<u8>,
			erc20_token_address: H160,
			mapping_token_hash: Hash,
			sync_begin_block_heigh: U256,
			rpc_urls: RpcUrl,
		) {
			let _signer = ensure_signed(origin)?;
			ensure!(Self::erc20_token_name().is_none(), "Already initialized");
			ensure!(Self::erc20_token_address().is_none(), "Already initialized");
			ensure!(Self::mapping_token_hash().is_none(), "Already initialized");
			ensure!(Self::sync_begin_block_heigh().is_none(), "Already initialized");
			ensure!(Self::rpc_urls().is_none(), "Already initialized");


			<Erc20TokenName>::set(Some(erc20_token_name));
			<Erc20TokenAddress>::set(Some(erc20_token_address));
			<MappingTokenHash>::set(Some(mapping_token_hash));
			<SyncBeginBlockHeight>::set(Some(sync_begin_block_heigh));
			<RpcUrls>::set(Some(rpc_urls));
		}

		#[weight = 0]
		pub fn add_erc20_transfers(
			origin,
			transfers: Vec<TransferInfo>,
		) {
			let _signer = ensure_signed(origin)?;

			let index: u64 = 0;
			for transfer in transfers {
				let erc20_transfer: TransferInfo = rlp::decode(transfer.as_slice()).unwrap();
				if let Some(sync_begin_block_heigh) = Self::infos(Self::sync_begin_block_heigh()) {
					let block_number = U256::from(transfer.block_number);
					let tx_hash = H256::from(transfer.tx_hash);

					if block_number > sync_begin_block_heigh {
						// Record full information about this header.
						<TxHashTransferList>::insert(tx_hash, index, transfer.clone());
						<BlockTransferList>::insert(block_number, index, transfer.clone());
						index = index + 1;
					}

					Self::current_block_heigh(block_number);
				}
			}
		}

		/// Offchain Worker entry point.
		///
		/// By implementing `fn offchain_worker` within `decl_module!` you declare a new offchain
		/// worker.
		/// This function will be called when the node is fully synced and a new best block is
		/// succesfuly imported.
		/// Note that it's not guaranteed for offchain workers to run on EVERY block, there might
		/// be cases where some blocks are skipped, or for some the worker runs twice (re-orgs),
		/// so the code should be able to handle that.
		/// You can use `Local Storage` API to coordinate runs of the worker.
		fn offchain_worker(block_number: T::BlockNumber) {
			// It's a good idea to add logs to your offchain workers.
			// Using the `frame_support::debug` module you have access to the same API exposed by
			// the `log` crate.
			// Note that having logs compiled to WASM may cause the size of the blob to increase
			// significantly. You can use `RuntimeDebug` custom derive to hide details of the types
			// in WASM or use `debug::native` namespace to produce logs only when the worker is
			// running natively.
			debug::native::info!("Hello World from offchain workers!");
			let sync_block_number = Self::last_block_number()
			let transfer_infos = Self::fetch_etherscan_transfers(sync_block_number).unwrap();

			let call = if Self::initialized() {
				if sync_block_number > Self::last_block_number() {
					debug::native::info!("Adding erc20 transfer at block number #: {:?}!", sync_block_number);
					Some(Call::add_erc20_transfers(transfer_infos.clone()))
				} else {
					debug::native::info!("Skipping adding #: {:?}, already added!", sync_block_number);
					None
				}
			};

			if let Some(c) = call {
			    let result = signer.send_signed_transaction(|_acct| c.clone());
			    // Display error if the signed tx fails.
			    if let Some((acc, res)) = result {
			        if res.is_err() {
			            debug::error!("failure: offchain_signed_tx: tx sent: {:?}", acc.id);
			        }
			        // Transaction is sent successfully
			    }
			}

			// Since off-chain workers are just part of the runtime code, they have direct access
			// to the storage and other included pallets.
			//
			// We can easily import `frame_system` and retrieve a block hash of the parent block.
			// let parent_hash = <system::Module<T>>::block_hash(block_number - 1.into());
			// debug::debug!("Current block: {:?} (parent hash: {:?})", block_number, parent_hash);
		}
	}
}

fn hex_to_bytes(v: &Vec<char>) -> Result<Vec<u8>, hex::FromHexError> {
	let mut vec = v.clone();

	// remove 0x prefix
	if vec.len() >= 2 && vec[0] == '0' && vec[1] == 'x' {
		vec.drain(0..2);
	}

	// add leading 0 if odd length
	if vec.len() % 2 != 0 {
		vec.insert(0, '0');
	}
	let vec_u8 = vec.iter().map(|c| *c as u8).collect::<Vec<u8>>();
	hex::decode(&vec_u8[..])
}

impl<T: Trait> Module<T> {
	pub fn initialized() -> bool {
		Self::erc20_token_address().is_some()
	}

	fn fetch_etherscan_transfers(block_number: U256) -> Result<types::BlockHeader, http::Error> {
		// Make a post request to etherscan
		let url = format!("https://api-cn.etherscan.com/api?module=account&action=tokentx&contractaddress=0x9f8f72aa9304c8b593d555f12ef6589cc3a579a2&startblock={}&endblock={}&sort=asc&apikey={}", block_number, block_number, "YourApiKeyToken");
		let request: http::Request = http::Request::get(url);
		let pending = request.send().unwrap();

		// wait indefinitely for response (TODO: timeout)
		let mut response = pending.wait().unwrap();
		let headers = response.headers().into_iter();
		assert_eq!(headers.current(), None);

		// and collect the body
		let body = response.body().collect::<Vec<u8>>();
		let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
			debug::warn!("No UTF8 body");
			http::Error::Unknown
		}).unwrap();
		// decode JSON into object
		let val: JsonValue = lite_json::parse_json(&body_str).unwrap();
		let header: Vec<TransferInfo> = Self::json_to_rlp(val);
		Ok(header)
	}

	pub fn json_to_rlp(json: JsonValue) -> Vec<TransferInfo> {
		// get { "result": VAL }
		// get { "status":"1","message":"OK","result":[{"blockNumber":"4620855","timeStamp":"1511634257","hash":"0x5c9b0f9c6c32d2690771169ec62dd648fef7bce3d45fe8a6505d99fdcbade27a","nonce":"5417","blockHash":"0xee385ac028bb7d8863d70afa02d63181894e0b2d51b99c0c525ef24538c44c24","from":"0x731c6f8c754fa404cfcc2ed8035ef79262f65702","contractAddress":"0x9f8f72aa9304c8b593d555f12ef6589cc3a579a2","to":"0x642ae78fafbb8032da552d619ad43f1d81e4dd7c","value":"1000000000000000000000000","tokenName":"Maker","tokenSymbol":"MKR","tokenDecimal":"18","transactionIndex":"55","gas":"3000000","gasPrice":"1000000000","gasUsed":"1594668","cumulativeGasUsed":"4047394","input":"deprecated","confirmations":"6783890"},{"blockNumber":"4621053","timeStamp":"1511636973","hash":"0x84877a2c8274c8d773b023e31cc74d9705790a1199f4f2127e25fc031f3eabab","nonce":"5419","blockHash":"0x4cc74a0b08e97e0cf8763b5e8d86fcd704df95b5c337ee57f82a6bc4d834fe2f","from":"0x642ae78fafbb8032da552d619ad43f1d81e4dd7c","contractAddress":"0x9f8f72aa9304c8b593d555f12ef6589cc3a579a2","to":"0x00daa9a2d88bed5a29a6ca93e0b7d860cd1d403f","value":"1000000000000000000","tokenName":"Maker","tokenSymbol":"MKR","tokenDecimal":"18","transactionIndex":"11","gas":"1223199","gasPrice":"1000000000","gasUsed":"92759","cumulativeGasUsed":"3844611","input":"deprecated","confirmations":"6783692"},{"blockNumber":"4621065","timeStamp":"1511637186","hash":"0x5313c5bf12d0441b50a9b82e11961c43ff2d645a5cd8ac0aa5a7f5c2b73d27e3","nonce":"5421","blockHash":"0x46437d28f167882af4440143ab6fd914cb5401f7351af2cbaffce23cdfd49ebd","from":"0x00daa9a2d88bed5a29a6ca93e0b7d860cd1d403f","contractAddress":"0x9f8f72aa9304c8b593d555f12ef6589cc3a579a2","to":"0x642ae78fafbb8032da552d619ad43f1d81e4dd7c","value":"1000000000000000000","tokenName":"Maker","tokenSymbol":"MKR","tokenDecimal":"18","transactionIndex":"35","gas":"187069","gasPrice":"1000000000","gasUsed":"52152","cumulativeGasUsed":"1107035","input":"deprecated","confirmations":"6783680"},{"blockNumber":"4621088","timeStamp":"1511637525","hash":"0x78e5963677a512b82a4a97333d6faf31253faa7e8bfa45394dbf57890fd665d1","nonce":"5425","blockHash":"0x476ec249441f5954debc3a5b000fc631ede07421d40b4c73fd087dfaa9d7f836","from":"0x642ae78fafbb8032da552d619ad43f1d81e4dd7c","contractAddress":"0x9f8f72aa9304c8b593d555f12ef6589cc3a579a2","to":"0x00daa9a2d88bed5a29a6ca93e0b7d860cd1d403f","value":"1000000000000000000","tokenName":"Maker","tokenSymbol":"MKR","tokenDecimal":"18","transactionIndex":"30","gas":"212761","gasPrice":"1000000000","gasUsed":"92759","cumulativeGasUsed":"1215572","input":"deprecated","confirmations":"6783657"}]}
		let vec_obj = match json {
			JsonValue::Object(obj) => {
				obj.into_iter()
					.find(|(k, _)| k.iter().map(|c| *c as u8).collect::<Vec<u8>>() == b"result".to_vec())
					.and_then(|v| {
						match v.1 {
							JsonValue::Array(transfers) => Some(transfers),
							_ => None,
						}
					})
			},
			_ => None
		};
		let transfers = match vec_obj {
			Some(value) => value,
			None => vec![],
		};
		let mut transfer_info_list = vec!();
		for transfer in transfers {
			// debug::native::info!("Decoding block_number!");
			let decoded_block_number_hex = Self::extract_property_from_transfer(transfer.clone(), b"blockNumber".to_vec());
			let block_number = U256::from_big_endian(&decoded_block_number_hex[..]).as_u64();

			let decoded_time_stamp_hex = Self::extract_property_from_transfer(transfer.clone(), b"timeStamp".to_vec());
			let time_stamp = U256::from_big_endian(&decoded_time_stamp_hex[..]).as_u64();

			let decoded_hash_hex = Self::extract_property_from_transfer(block.clone(), b"hash".to_vec());
			let mut temp_hash = [0; 32];
			for i in 0..decoded_hash_hex.len() {
				temp_hash[i] = decoded_hash_hex[i];
			}
			let hash = H256::from(temp_hash);

			let decoded_nonce_hex = Self::extract_property_from_transfer(transfer.clone(), b"nonce".to_vec());
			let time_stamp = U256::from_big_endian(&decoded_time_stamp_hex[..]).as_u64();

			let decoded_nonce_hex = Self::extract_property_from_transfer(transfer.clone(), b"nonce".to_vec());
			let nonce = U256::from_big_endian(&decoded_nonce_hex[..]).as_u64();

			let decoded_block_hash_hex = Self::extract_property_from_transfer(block.clone(), b"blockHash".to_vec());
			let mut temp_hash = [0; 32];
			for i in 0..decoded_hash_hex.len() {
				temp_hash[i] = decoded_hash_hex[i];
			}
			let block_hash = H256::from(temp_hash);

			// debug::native::info!("Decoding from_address!");
			let decoded_from_address_hex = Self::extract_property_from_transfer(block.clone(), b"from".to_vec());
			let mut temp_from = [0; 32];
			for i in 0..decoded_from_address_hex.len() {
				temp_from[i] = decoded_from_address_hex[i];
			}
			let from_address = H160::from(temp_from);

			// debug::native::info!("Decoding to_address!");
			let decoded_to_address_hex = Self::extract_property_from_transfer(block.clone(), b"to".to_vec());
			let mut temp_to = [0; 32];
			for i in 0..decoded_to_address_hex.len() {
				temp_to[i] = decoded_to_address_hex[i];
			}
			let to_address = H160::from(temp_to);

			// debug::native::info!("Decoding contract_address!");
			let decoded_contract_address_hex = Self::extract_property_from_transfer(block.clone(), b"contractAddress".to_vec());
			let mut temp_contract_address = [0; 32];
			for i in 0..decoded_contract_address_hex.len() {
				temp_contract_address[i] = decoded_contract_address_hex[i];
			}
			let contract_address = H160::from(temp_contract_address);

			// debug::native::info!("Decoding value!");
			let decoded_value_hex = Self::extract_property_from_transfer(transfer.clone(), b"value".to_vec());
			let value = U256::from_big_endian(&decoded_value_hex[..]).as_u64();

			// debug::native::info!("Decoding tokenName!");
			let decoded_token_name_hex = Self::extract_property_from_transfer(transfer.clone(), b"tokenName".to_vec());

			// debug::native::info!("Decoding tokenSymbol!");
			let decoded_token_symbol_hex = Self::extract_property_from_transfer(transfer.clone(), b"tokenSymbol".to_vec());

			// debug::native::info!("Decoding token_decimal!");
			let decoded_token_decimal_hex = Self::extract_property_from_transfer(transfer.clone(), b"tokenDecimal".to_vec());
			let token_decimal = U256::from_big_endian(&decoded_token_decimal_hex[..]).as_u64();

			// debug::native::info!("Decoding transaction_index!");
			let decoded_transaction_index_hex = Self::extract_property_from_transfer(transfer.clone(), b"transactionIndex".to_vec());
			let transaction_index = U256::from_big_endian(&decoded_transaction_index_hex[..]).as_u64();

			// debug::native::info!("Decoding gas!");
			let decoded_gas_hex = Self::extract_property_from_transfer(transfer.clone(), b"gas".to_vec());
			let gas = U256::from_big_endian(&decoded_gas_hex[..]).as_u64();

			// debug::native::info!("Decoding gasPrice!");
			let decoded_gas_price_hex = Self::extract_property_from_transfer(transfer.clone(), b"gasPrice".to_vec());
			let gas_price = U256::from_big_endian(&decoded_gas_price_hex[..]).as_u64();

			// debug::native::info!("Decoding gas_used!");
			let decoded_gas_used_hex = Self::extract_property_from_transfer(transfer.clone(), b"gasUsed".to_vec());
			let gas_used = U256::from_big_endian(&decoded_gas_used_hex[..]).as_u64();

			// debug::native::info!("Decoding cumulativeGasUsed!");
			let decoded_cumulative_gas_used_hex = Self::extract_property_from_transfer(transfer.clone(), b"cumulativeGasUsed".to_vec());
			let cumulative_gas_used = U256::from_big_endian(&decoded_cumulative_gas_used_hex[..]).as_u64();

			// debug::native::info!("Decoding confirmations!");
			let decoded_confirmations_hex = Self::extract_property_from_transfer(transfer.clone(), b"confirmations".to_vec());
			let confirmations = U256::from_big_endian(&decoded_confirmations_hex[..]).as_u64();

			let transfer_info = TransferInfo {
				block_number: block_number,
				time_stamp: time_stamp,
				tx_hash: hash,
				nonce: nonce,
				block_hash: block_hash,
				from_address: from_address,
				to_address: to_address,
				contract_address: contract_address,
				value: value,
				token_name: decoded_token_name_hex,
				token_symbol: decoded_token_symbol_hex,
				token_decimal: token_decimal,
				transaction_index: transaction_index,
				gas: gas,
				gas_price: gas_price,
				gas_used: gas_used,
				cumulative_gas_used: cumulative_gas_used,
				confirmations: confirmations,
			};

			transfer_info_list.push(transfer_info);
		};
		transfer_info_list
	}

	pub fn extract_property_from_transfer(transfer: Option<Vec<(Vec<char>, JsonValue)>>, property: Vec<u8>) -> Vec<u8> {
		let extracted_hex: Vec<char> = transfer.unwrap().into_iter()
			.find(|(k, _)| k.iter().map(|c| *c as u8).collect::<Vec<u8>>() == property)
			.and_then(|v| match v.1 {
				JsonValue::String(n) => Some(n),
				_ => None,
			})
			.unwrap();
		let decoded_hex = hex_to_bytes(&extracted_hex).unwrap();
		decoded_hex
	}


}

#[allow(deprecated)] // ValidateUnsigned
impl<T: Trait> frame_support::unsigned::ValidateUnsigned for Module<T> {
	type Call = Call<T>;

	/// Validate unsigned call to this module.
	///
	/// By default unsigned transactions are disallowed, but implementing the validator
	/// here we make sure that some particular calls (the ones produced by offchain worker)
	/// are being whitelisted and marked as valid.
	fn validate_unsigned(
		_source: TransactionSource,
		_call: &Self::Call,
	) -> TransactionValidity {
		ValidTransaction::with_tag_prefix("EtherscanWorker")
		// We set base priority to 2**20 and hope it's included before any other
		// transactions in the pool. Next we tweak the priority depending on how much
		// it differs from the current average. (the more it differs the more priority it
		// has).
		.priority(T::UnsignedPriority::get())
		// The transaction is only valid for next 5 blocks. After that it's
		// going to be revalidated by the pool.
		.longevity(5)
		// It's fine to propagate that transaction to other peers, which means it can be
		// created even by nodes that don't produce blocks.
		// Note that sometimes it's better to keep it for yourself (if you are the block
		// producer), since for instance in some schemes others may copy your solution and
		// claim a reward.
		.propagate(true)
		.build()
	}
}