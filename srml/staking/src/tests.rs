// Copyright 2017-2019 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

//! Tests for the module.

#![cfg(test)]

use super::*;
use runtime_io::with_externalities;
use phragmen;
use primitives::Perquintill;
use srml_support::{assert_ok, assert_noop, EnumerableStorageMap};
use mock::{Balances, Session, Staking, System, Timestamp, Test, ExtBuilder, Origin};
use srml_support::traits::Currency;

#[test]
fn basic_setup_works() {
	// Verifies initial conditions of mock
	with_externalities(&mut ExtBuilder::default()
		.build(),
	|| {
		assert_eq!(Staking::bonded(&11), Some(10)); // Account 11 is stashed and locked, and account 10 is the controller
		assert_eq!(Staking::bonded(&21), Some(20)); // Account 21 is stashed and locked, and account 20 is the controller
		assert_eq!(Staking::bonded(&1), None);		// Account 1 is not a stashed

		// Account 10 controls the stash from account 11, which is 100 * balance_factor units
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000, active: 1000, unlocking: vec![] }));
		// Account 20 controls the stash from account 21, which is 200 * balance_factor units
		assert_eq!(Staking::ledger(&20), Some(StakingLedger { stash: 21, total: 1000, active: 1000, unlocking: vec![] }));
		// Account 1 does not control any stash
		assert_eq!(Staking::ledger(&1), None);

		// ValidatorPrefs are default, thus unstake_threshold is 3, other values are default for their type
		assert_eq!(<Validators<Test>>::enumerate().collect::<Vec<_>>(), vec![
			(21, ValidatorPrefs { unstake_threshold: 3, validator_payment: 0 }),
			(11, ValidatorPrefs { unstake_threshold: 3, validator_payment: 0 })
		]);

		// Account 100 is the default nominator
		assert_eq!(Staking::ledger(100), Some(StakingLedger { stash: 101, total: 500, active: 500, unlocking: vec![] }));
		assert_eq!(Staking::nominators(101), vec![11, 21]);

		// Account 10 is exposed by 1000 * balance_factor from their own stash in account 11 + the default nominator vote
		assert_eq!(Staking::stakers(11), Exposure { total: 1125, own: 1000, others: vec![ IndividualExposure { who: 101, value: 125 }] });
		// Account 20 is exposed by 1000 * balance_factor from their own stash in account 21 + the default nominator vote
		assert_eq!(Staking::stakers(21), Exposure { total: 1375, own: 1000, others: vec![ IndividualExposure { who: 101, value: 375 }] });

		// The number of validators required.
		assert_eq!(Staking::validator_count(), 2);

		// Initial Era and session
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 0);

		// initial rewards
		assert_eq!(Staking::current_session_reward(), 10);

		// initial slot_stake
		assert_eq!(Staking::slot_stake(),  1125); // Naive
		// assert_eq!(Staking::slot_stake(),  1250); // Post-process

		// initial slash_count of validators
		assert_eq!(Staking::slash_count(&11), 0);
		assert_eq!(Staking::slash_count(&21), 0);
	});
}

#[test]
fn no_offline_should_work() {
	// Test the staking module works when no validators are offline
	with_externalities(&mut ExtBuilder::default().build(),
	|| {
		// Slashing begins for validators immediately if found offline
		assert_eq!(Staking::offline_slash_grace(), 0);
		// Account 10 has not been reported offline
		assert_eq!(Staking::slash_count(&10), 0);
		// Account 10 has `balance_factor` free balance
		assert_eq!(Balances::free_balance(&10), 1);
		// Nothing happens to Account 10, as expected
		assert_eq!(Staking::slash_count(&10), 0);
		assert_eq!(Balances::free_balance(&10), 1);
		// New era is not being forced
		assert!(Staking::forcing_new_era().is_none());
	});
}

#[test]
fn invulnerability_should_work() {
	// Test that users can be invulnerable from slashing and being kicked
	with_externalities(&mut ExtBuilder::default().build(),
	|| {
		// Make account 11 invulnerable
		assert_ok!(Staking::set_invulnerables(vec![11]));
		// Give account 11 some funds
		let _ = Balances::make_free_balance_be(&11, 70);
		// There is no slash grace -- slash immediately.
		assert_eq!(Staking::offline_slash_grace(), 0);
		// Account 11 has not been slashed
		assert_eq!(Staking::slash_count(&11), 0);
		// Account 11 has the 70 funds we gave it above
		assert_eq!(Balances::free_balance(&11), 70);
		// Account 11 should be a validator
		assert!(<Validators<Test>>::exists(&11));

		// Set account 11 as an offline validator with a large number of reports
		// Should exit early if invulnerable
		Staking::on_offline_validator(10, 100);

		// Show that account 11 has not been touched
		assert_eq!(Staking::slash_count(&11), 0);
		assert_eq!(Balances::free_balance(&11), 70);
		assert!(<Validators<Test>>::exists(&11));
		// New era not being forced
		// NOTE: new era is always forced once slashing happens -> new validators need to be chosen.
		assert!(Staking::forcing_new_era().is_none());
	});
}

#[test]
fn offline_should_slash_and_kick() {
	// Test that an offline validator gets slashed and kicked
	with_externalities(&mut ExtBuilder::default().build(), || {
		// Give account 10 some balance
		let _ = Balances::make_free_balance_be(&11, 1000);
		// Confirm account 10 is a validator
		assert!(<Validators<Test>>::exists(&11));
		// Validators get slashed immediately
		assert_eq!(Staking::offline_slash_grace(), 0);
		// Unstake threshold is 3
		assert_eq!(Staking::validators(&11).unstake_threshold, 3);
		// Account 10 has not been slashed before
		assert_eq!(Staking::slash_count(&11), 0);
		// Account 10 has the funds we just gave it
		assert_eq!(Balances::free_balance(&11), 1000);
		// Report account 10 as offline, one greater than unstake threshold
		Staking::on_offline_validator(10, 4);
		// Confirm user has been reported
		assert_eq!(Staking::slash_count(&11), 4);
		// Confirm balance has been reduced by 2^unstake_threshold * offline_slash() * amount_at_stake.
		let slash_base = Staking::offline_slash() * Staking::stakers(11).total;
		assert_eq!(Balances::free_balance(&11), 1000 - 2_u64.pow(3) * slash_base);
		// Confirm account 10 has been removed as a validator
		assert!(!<Validators<Test>>::exists(&11));
		// A new era is forced due to slashing
		assert!(Staking::forcing_new_era().is_some());
	});
}

#[test]
fn offline_grace_should_delay_slashing() {
	// Tests that with grace, slashing is delayed
	with_externalities(&mut ExtBuilder::default().build(), || {
		// Initialize account 10 with balance
		let _ = Balances::make_free_balance_be(&11, 70);
		// Verify account 11 has balance
		assert_eq!(Balances::free_balance(&11), 70);

		// Set offline slash grace
		let offline_slash_grace = 1;
		assert_ok!(Staking::set_offline_slash_grace(offline_slash_grace));
		assert_eq!(Staking::offline_slash_grace(), 1);

		// Check unstaked_threshold is 3 (default)
		let default_unstake_threshold = 3;
		assert_eq!(Staking::validators(&11), ValidatorPrefs { unstake_threshold: default_unstake_threshold, validator_payment: 0 });

		// Check slash count is zero
		assert_eq!(Staking::slash_count(&11), 0);

		// Report account 10 up to the threshold
		Staking::on_offline_validator(10, default_unstake_threshold as usize + offline_slash_grace as usize);
		// Confirm slash count
		assert_eq!(Staking::slash_count(&11), 4);

		// Nothing should happen
		assert_eq!(Balances::free_balance(&11), 70);

		// Report account 10 one more time
		Staking::on_offline_validator(10, 1);
		assert_eq!(Staking::slash_count(&11), 5);
		// User gets slashed
		assert_eq!(Balances::free_balance(&11), 0);
		// New era is forced
		assert!(Staking::forcing_new_era().is_some());
	});
}


#[test]
fn max_unstake_threshold_works() {
	// Tests that max_unstake_threshold gets used when prefs.unstake_threshold is large
	with_externalities(&mut ExtBuilder::default().build(), || {
		const MAX_UNSTAKE_THRESHOLD: u32 = 10;
		// Two users with maximum possible balance
		let _ = Balances::make_free_balance_be(&11, u64::max_value());
		let _ = Balances::make_free_balance_be(&21, u64::max_value());

		// Give them full exposure as a staker
		<Stakers<Test>>::insert(&11, Exposure { total: 1000000, own: 1000000, others: vec![]});
		<Stakers<Test>>::insert(&21, Exposure { total: 2000000, own: 2000000, others: vec![]});

		// Check things are initialized correctly
		assert_eq!(Balances::free_balance(&11), u64::max_value());
		assert_eq!(Balances::free_balance(&21), u64::max_value());
		assert_eq!(Staking::offline_slash_grace(), 0);
		// Account 10 will have max unstake_threshold
		assert_ok!(Staking::validate(Origin::signed(10), ValidatorPrefs {
			unstake_threshold: MAX_UNSTAKE_THRESHOLD,
			validator_payment: 0,
		}));
		// Account 20 could not set their unstake_threshold past 10
		assert_noop!(Staking::validate(Origin::signed(20), ValidatorPrefs {
			unstake_threshold: MAX_UNSTAKE_THRESHOLD + 1,
			validator_payment: 0}),
			"unstake threshold too large"
		);
		// Give Account 20 unstake_threshold 11 anyway, should still be limited to 10
		<Validators<Test>>::insert(21, ValidatorPrefs {
			unstake_threshold: MAX_UNSTAKE_THRESHOLD + 1,
			validator_payment: 0,
		});

		<OfflineSlash<Test>>::put(Perbill::from_fraction(0.0001));

		// Report each user 1 more than the max_unstake_threshold
		Staking::on_offline_validator(10, MAX_UNSTAKE_THRESHOLD as usize + 1);
		Staking::on_offline_validator(20, MAX_UNSTAKE_THRESHOLD as usize + 1);

		// Show that each balance only gets reduced by 2^max_unstake_threshold times 10%
		// of their total stake.
		assert_eq!(Balances::free_balance(&11), u64::max_value() - 2_u64.pow(MAX_UNSTAKE_THRESHOLD) * 100);
		assert_eq!(Balances::free_balance(&21), u64::max_value() - 2_u64.pow(MAX_UNSTAKE_THRESHOLD) * 200);
	});
}

#[test]
fn slashing_does_not_cause_underflow() {
	// Tests that slashing more than a user has does not underflow
	with_externalities(&mut ExtBuilder::default().build(), || {
		// Verify initial conditions
		assert_eq!(Balances::free_balance(&11), 1000);
		assert_eq!(Staking::offline_slash_grace(), 0);

		// Set validator preference so that 2^unstake_threshold would cause overflow (greater than 64)
		<Validators<Test>>::insert(11, ValidatorPrefs {
			unstake_threshold: 10,
			validator_payment: 0,
		});

		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		// Should not panic
		Staking::on_offline_validator(10, 100);
		// Confirm that underflow has not occurred, and account balance is set to zero
		assert_eq!(Balances::free_balance(&11), 0);
	});
}


#[test]
fn rewards_should_work() {
	// should check that:
	// * rewards get recorded per session
	// * rewards get paid per Era
	// * Check that nominators are also rewarded
	with_externalities(&mut ExtBuilder::default()
		.session_length(3)
		.sessions_per_era(3)
	.build(),
	|| {
		let delay = 0;
		// this test is only in the scope of one era. Since this variable changes
		// at the last block/new era, we'll save it.
		let session_reward = 10;

		// Set payee to controller
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Controller));

		// Initial config should be correct
		assert_eq!(Staking::era_length(), 9);
		assert_eq!(Staking::sessions_per_era(), 3);
		assert_eq!(Staking::last_era_length_change(), 0);
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 0);
		assert_eq!(Staking::current_session_reward(), 10);

		// check the balance of a validator accounts.
		assert_eq!(Balances::total_balance(&11), 1000);
		// and the nominator (to-be)
		let _ = Balances::make_free_balance_be(&2, 500);
		assert_eq!(Balances::total_balance(&2), 500);

		// add a dummy nominator.
		// NOTE: this nominator is being added 'manually'. a Further test (nomination_and_reward..) will add it via '.nominate()'
		<Stakers<Test>>::insert(&11, Exposure {
			own: 500, // equal division indicates that the reward will be equally divided among validator and nominator.
			total: 1000,
			others: vec![IndividualExposure {who: 2, value: 500 }]
		});

		<Payee<Test>>::insert(&2, RewardDestination::Stash);
		assert_eq!(Staking::payee(2), RewardDestination::Stash);
		assert_eq!(Staking::payee(11), RewardDestination::Controller);

		let mut block = 3;
		// Block 3 => Session 1 => Era 0
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5);	// on time.
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 1);

		// session triggered: the reward value stashed should be 10 -- defined in ExtBuilder genesis.
		assert_eq!(Staking::current_session_reward(), session_reward);
		assert_eq!(Staking::current_era_reward(), session_reward);

		block = 6; // Block 6 => Session 2 => Era 0
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5 + delay);	// a little late.
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 2);

		// session reward is the same,
		assert_eq!(Staking::current_session_reward(), session_reward);
		// though 2 will be deducted while stashed in the era reward due to delay
		assert_eq!(Staking::current_era_reward(), 2*session_reward - delay);

		block = 9; // Block 9 => Session 3 => Era 1
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5);  // back to being on time. no delays
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 1);
		assert_eq!(Session::current_index(), 3);

		assert_eq!(Balances::total_balance(&10), 1 + (3*session_reward - delay)/2);
		assert_eq!(Balances::total_balance(&2), 500 + (3*session_reward - delay)/2);
	});
}

#[test]
fn multi_era_reward_should_work() {
	// should check that:
	// The value of current_session_reward is set at the end of each era, based on
	// slot_stake and session_reward. Check and verify this.
	with_externalities(&mut ExtBuilder::default()
		.session_length(3)
		.sessions_per_era(3)
		.nominate(false)
		.build(),
	|| {
		let delay = 0;
		let session_reward = 10;

		// This is set by the test config builder.
		assert_eq!(Staking::current_session_reward(), session_reward);

		// check the balance of a validator accounts.
		assert_eq!(Balances::total_balance(&10), 1);

		// Set payee to controller
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Controller));

		let mut block = 3;
		// Block 3 => Session 1 => Era 0
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5);	// on time.
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 1);

		// session triggered: the reward value stashed should be 10 -- defined in ExtBuilder genesis.
		assert_eq!(Staking::current_session_reward(), session_reward);
		assert_eq!(Staking::current_era_reward(), session_reward);

		block = 6; // Block 6 => Session 2 => Era 0
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5 + delay);	// a little late.
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 2);

		assert_eq!(Staking::current_session_reward(), session_reward);
		assert_eq!(Staking::current_era_reward(), 2*session_reward - delay);

		block = 9; // Block 9 => Session 3 => Era 1
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5);  // back to being punktlisch. no delayss
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 1);
		assert_eq!(Session::current_index(), 3);

		// 1 + sum of of the session rewards accumulated
		let recorded_balance = 1 + 3*session_reward - delay;
		assert_eq!(Balances::total_balance(&10), recorded_balance);

		// the reward for next era will be: session_reward * slot_stake
		let new_session_reward = Staking::session_reward() * Staking::slot_stake();
		assert_eq!(Staking::current_session_reward(), new_session_reward);

		// fast forward to next era:
		block=12;System::set_block_number(block);Timestamp::set_timestamp(block*5);Session::check_rotate_session(System::block_number());
		block=15;System::set_block_number(block);Timestamp::set_timestamp(block*5);Session::check_rotate_session(System::block_number());

		// intermediate test.
		assert_eq!(Staking::current_era_reward(), 2*new_session_reward);

		// new era is triggered here.
		block=18;System::set_block_number(block);Timestamp::set_timestamp(block*5);Session::check_rotate_session(System::block_number());

		// pay time
		assert_eq!(Balances::total_balance(&10), 3*new_session_reward + recorded_balance);
	});
}

#[test]
fn staking_should_work() {
	// should test:
	// * new validators can be added to the default set
	// * new ones will be chosen per era
	// * either one can unlock the stash and back-down from being a validator via `chill`ing.
	with_externalities(&mut ExtBuilder::default()
		.sessions_per_era(3)
		.nominate(false)
		.fare(false) // to give 20 more staked value
		.build(),
	|| {
		// remember + compare this along with the test.
		assert_eq!(Session::validators(), vec![20, 10]);

		assert_ok!(Staking::set_bonding_duration(2));
		assert_eq!(Staking::bonding_duration(), 2);

		// put some money in account that we'll use.
		for i in 1..5 { let _ = Balances::make_free_balance_be(&i, 2000); }

		// --- Block 1:
		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 0);

		// add a new candidate for being a validator. account 3 controlled by 4.
		assert_ok!(Staking::bond(Origin::signed(3), 4, 1500, RewardDestination::Controller));
		assert_ok!(Staking::validate(Origin::signed(4), ValidatorPrefs::default()));

		// No effects will be seen so far.
		assert_eq!(Session::validators(), vec![20, 10]);

		// --- Block 2:
		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 0);

		// No effects will be seen so far. Era has not been yet triggered.
		assert_eq!(Session::validators(), vec![20, 10]);


		// --- Block 3: the validators will now change.
		System::set_block_number(3);
		Session::check_rotate_session(System::block_number());

		// 2 only voted for 4 and 20
		assert_eq!(Session::validators().len(), 2);
		assert_eq!(Session::validators(), vec![20, 4]);
		assert_eq!(Staking::current_era(), 1);


		// --- Block 4: Unstake 4 as a validator, freeing up the balance stashed in 3
		System::set_block_number(4);
		Session::check_rotate_session(System::block_number());

		// 4 will chill
		Staking::chill(Origin::signed(4)).unwrap();

		// nothing should be changed so far.
		assert_eq!(Session::validators(), vec![20, 4]);
		assert_eq!(Staking::current_era(), 1);


		// --- Block 5: nothing. 4 is still there.
		System::set_block_number(5);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::validators(), vec![20, 4]);
		assert_eq!(Staking::current_era(), 1);


		// --- Block 6: 4 will not be a validator.
		System::set_block_number(6);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 2);
		assert_eq!(Session::validators().contains(&4), false);
		assert_eq!(Session::validators(), vec![20, 10]);

		// Note: the stashed value of 4 is still lock
		assert_eq!(Staking::ledger(&4), Some(StakingLedger { stash: 3, total: 1500, active: 1500, unlocking: vec![] }));
		// e.g. it cannot spend more than 500 that it has free from the total 2000
		assert_noop!(Balances::reserve(&3, 501), "account liquidity restrictions prevent withdrawal");
		assert_ok!(Balances::reserve(&3, 409));
	});
}

#[test]
fn less_than_needed_candidates_works() {
	// Test the situation where the number of validators are less than `ValidatorCount` but more than <MinValidators>
	// The expected behavior is to choose all the candidates that have some vote.
	with_externalities(&mut ExtBuilder::default()
		.minimum_validator_count(1)
		.validator_count(3)
		.nominate(false)
		.build(),
	|| {
		assert_eq!(Staking::era_length(), 1);
		assert_eq!(Staking::validator_count(), 3);
		assert_eq!(Staking::minimum_validator_count(), 1);

		// initial validators
		assert_eq!(Session::validators(), vec![20, 10]);

		// 10 and 20 are now valid candidates.
		// trigger era
		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 1);

		// both validators will be chosen again. NO election algorithm is even executed.
		assert_eq!(Session::validators(), vec![20, 10]);

		// But the exposure is updated in a simple way. No external votes exists. This is purely self-vote.
		assert_eq!(Staking::stakers(10).others.iter().map(|e| e.who).collect::<Vec<BalanceOf<Test>>>(), vec![]);
		assert_eq!(Staking::stakers(20).others.iter().map(|e| e.who).collect::<Vec<BalanceOf<Test>>>(), vec![]);
	});
}

#[test]
fn no_candidate_emergency_condition() {
	// Test the situation where the number of validators are less than `ValidatorCount` and less than <MinValidators>
	// The expected behavior is to choose all candidates from the previous era.
	with_externalities(&mut ExtBuilder::default()
		.minimum_validator_count(10)
		.validator_count(15)
		.validator_pool(true)
		.nominate(false)
		.build(),
	|| {
		assert_eq!(Staking::era_length(), 1);
		assert_eq!(Staking::validator_count(), 15);

		// initial validators
		assert_eq!(Session::validators(), vec![10, 20, 30, 40]);

		// trigger era
		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 1);

		// No one nominates => no one has a proper vote => no change
		assert_eq!(Session::validators(), vec![10, 20, 30, 40]);
	});
}

#[test]
fn nominating_and_rewards_should_work() {
	// For now it tests a functionality which somehow overlaps with other tests:
	// the fact that the nominator is rewarded properly.
	//
	// PHRAGMEN OUTPUT: running this test with the reference impl gives:
	//
	// Votes  [('10', 1000, ['10']), ('20', 1000, ['20']), ('30', 1000, ['30']), ('40', 1000, ['40']), ('2', 1000, ['10', '20', '30']), ('4', 1000, ['10', '20', '40'])]
	// Sequential Phragmén gives
	// 10  is elected with stake  2200.0 and score  0.0003333333333333333
	// 20  is elected with stake  1800.0 and score  0.0005555555555555556

	// 10  has load  0.0003333333333333333 and supported
	// 10  with stake  1000.0
	// 20  has load  0.0005555555555555556 and supported
	// 20  with stake  1000.0
	// 30  has load  0 and supported
	// 30  with stake  0
	// 40  has load  0 and supported
	// 40  with stake  0
	// 2  has load  0.0005555555555555556 and supported
	// 10  with stake  600.0 20  with stake  400.0 30  with stake  0.0
	// 4  has load  0.0005555555555555556 and supported
	// 10  with stake  600.0 20  with stake  400.0 40  with stake  0.0

	// Sequential Phragmén with post processing gives
	// 10  is elected with stake  2000.0 and score  0.0003333333333333333
	// 20  is elected with stake  2000.0 and score  0.0005555555555555556

	// 10  has load  0.0003333333333333333 and supported
	// 10  with stake  1000.0
	// 20  has load  0.0005555555555555556 and supported
	// 20  with stake  1000.0
	// 30  has load  0 and supported
	// 30  with stake  0
	// 40  has load  0 and supported
	// 40  with stake  0
	// 2  has load  0.0005555555555555556 and supported
	// 10  with stake  400.0 20  with stake  600.0 30  with stake  0
	// 4  has load  0.0005555555555555556 and supported
	// 10  with stake  600.0 20  with stake  400.0 40  with stake  0.0

	with_externalities(&mut ExtBuilder::default()
		.nominate(false)
		.validator_pool(true)
		.build(),
	|| {
		// initial validators -- everyone is actually even.
		assert_eq!(Session::validators(), vec![40, 30]);

		// Set payee to controller
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Controller));
		assert_ok!(Staking::set_payee(Origin::signed(20), RewardDestination::Controller));
		assert_ok!(Staking::set_payee(Origin::signed(30), RewardDestination::Controller));
		assert_ok!(Staking::set_payee(Origin::signed(40), RewardDestination::Controller));

		// default reward for the first session.
		let session_reward = 10;
		assert_eq!(Staking::current_session_reward(), session_reward);

		// give the man some money
		let initial_balance = 1000;
		for i in [1, 2, 3, 4, 5, 10, 11, 20, 21].iter() {
			let _ = Balances::make_free_balance_be(i, initial_balance);
		}

		// record their balances.
		for i in 1..5 { assert_eq!(Balances::total_balance(&i), initial_balance); }

		// bond two account pairs and state interest in nomination.
		// 2 will nominate for 10, 20, 30
		assert_ok!(Staking::bond(Origin::signed(1), 2, 1000, RewardDestination::Controller));
		assert_ok!(Staking::nominate(Origin::signed(2), vec![11, 21, 31]));
		// 4 will nominate for 10, 20, 40
		assert_ok!(Staking::bond(Origin::signed(3), 4, 1000, RewardDestination::Controller));
		assert_ok!(Staking::nominate(Origin::signed(4), vec![11, 21, 41]));

		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 1);

		// 10 and 20 have more votes, they will be chosen by phragmen.
		assert_eq!(Session::validators(), vec![20, 10]);

		// OLD validators must have already received some rewards.
		assert_eq!(Balances::total_balance(&40), 1 + session_reward);
		assert_eq!(Balances::total_balance(&30), 1 + session_reward);

		// ------ check the staked value of all parties.

		// total expo of 10, with 1200 coming from nominators (externals), according to phragmen.
		assert_eq!(Staking::stakers(11).own, 1000);
		assert_eq!(Staking::stakers(11).total, 1000 + 800);
		// 2 and 4 supported 10, each with stake 600, according to phragmen.
		assert_eq!(Staking::stakers(11).others.iter().map(|e| e.value).collect::<Vec<BalanceOf<Test>>>(), vec![400, 400]);
		assert_eq!(Staking::stakers(11).others.iter().map(|e| e.who).collect::<Vec<BalanceOf<Test>>>(), vec![3, 1]);
		// total expo of 20, with 500 coming from nominators (externals), according to phragmen.
		assert_eq!(Staking::stakers(21).own, 1000);
		assert_eq!(Staking::stakers(21).total, 1000 + 1200);
		// 2 and 4 supported 20, each with stake 250, according to phragmen.
		assert_eq!(Staking::stakers(21).others.iter().map(|e| e.value).collect::<Vec<BalanceOf<Test>>>(), vec![600, 600]);
		assert_eq!(Staking::stakers(21).others.iter().map(|e| e.who).collect::<Vec<BalanceOf<Test>>>(), vec![3, 1]);

		// They are not chosen anymore
		assert_eq!(Staking::stakers(31).total, 0);
		assert_eq!(Staking::stakers(41).total, 0);


		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());
		// next session reward.
		let new_session_reward = Staking::session_reward() * Staking::slot_stake();
		// nothing else will happen, era ends and rewards are paid again,
		// it is expected that nominators will also be paid. See below

	// Nominator 2: has [400/1800 ~ 2/9 from 10] + [600/2200 ~ 3/11 from 20]'s reward. ==> 2/9 + 3/11
		assert_eq!(Balances::total_balance(&2), initial_balance + (2*new_session_reward/9 + 3*new_session_reward/11));
		// Nominator 4: has [400/1800 ~ 2/9 from 10] + [600/2200 ~ 3/11 from 20]'s reward. ==> 2/9 + 3/11
		assert_eq!(Balances::total_balance(&4), initial_balance + (2*new_session_reward/9 + 3*new_session_reward/11));

		// 10 got 800 / 1800 external stake => 8/18 =? 4/9 => Validator's share = 5/9
		assert_eq!(Balances::total_balance(&10), initial_balance + 5*new_session_reward/9) ;
		// 10 got 1200 / 2200 external stake => 12/22 =? 6/11 => Validator's share = 5/11
		assert_eq!(Balances::total_balance(&20), initial_balance + 5*new_session_reward/11);
	});
}

#[test]
fn nominators_also_get_slashed() {
	// A nominator should be slashed if the validator they nominated is slashed
	with_externalities(&mut ExtBuilder::default().nominate(false).build(), || {
		assert_eq!(Staking::era_length(), 1);
		assert_eq!(Staking::validator_count(), 2);
		// slash happens immediately.
		assert_eq!(Staking::offline_slash_grace(), 0);
		// Account 10 has not been reported offline
		assert_eq!(Staking::slash_count(&10), 0);
		// initial validators
		assert_eq!(Session::validators(), vec![20, 10]);
		<OfflineSlash<Test>>::put(Perbill::from_percent(12));

		// Set payee to controller
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Controller));

		// give the man some money.
		let initial_balance = 1000;
		for i in [1, 2, 3, 10].iter() {
			let _ = Balances::make_free_balance_be(i, initial_balance);
		}

		// 2 will nominate for 10
		let nominator_stake = 500;
		assert_ok!(Staking::bond(Origin::signed(1), 2, nominator_stake, RewardDestination::default()));
		assert_ok!(Staking::nominate(Origin::signed(2), vec![20, 10]));

		// new era, pay rewards,
		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		// Nominator stash didn't collect any.
		assert_eq!(Balances::total_balance(&2), initial_balance);

		// 10 goes offline
		Staking::on_offline_validator(10, 4);
		let expo = Staking::stakers(10);
		let slash_value = Staking::offline_slash() * expo.total * 2_u64.pow(3);
		let total_slash = expo.total.min(slash_value);
		let validator_slash = expo.own.min(total_slash);
		let nominator_slash = nominator_stake.min(total_slash - validator_slash);

		// initial + first era reward + slash
		assert_eq!(Balances::total_balance(&10), initial_balance + 10 - validator_slash);
		assert_eq!(Balances::total_balance(&2), initial_balance - nominator_slash);
		// Because slashing happened.
		assert!(Staking::forcing_new_era().is_some());
	});
}

#[test]
fn double_staking_should_fail() {
	// should test (in the same order):
	// * an account already bonded as controller CAN be reused as the controller of another account.
	// * an account already bonded as stash cannot be the controller of another account.
	// * an account already bonded as stash cannot nominate.
	// * an account already bonded as controller can nominate.
	with_externalities(&mut ExtBuilder::default()
		.sessions_per_era(2)
		.build(),
		|| {
			let arbitrary_value = 5;
			System::set_block_number(1);
			// 2 = controller, 1 stashed => ok
			assert_ok!(Staking::bond(Origin::signed(1), 2, arbitrary_value, RewardDestination::default()));
			// 4 = not used so far, 1 stashed => not allowed.
			assert_noop!(Staking::bond(Origin::signed(1), 4, arbitrary_value, RewardDestination::default()), "stash already bonded");
			// 1 = stashed => attempting to nominate should fail.
			assert_noop!(Staking::nominate(Origin::signed(1), vec![1]), "not a controller");
			// 2 = controller  => nominating should work.
			assert_ok!(Staking::nominate(Origin::signed(2), vec![1]));
		});
}

#[test]
fn double_controlling_should_fail() {
	// should test (in the same order):
	// * an account already bonded as controller CAN be reused as the controller of another account.
	// * an account already bonded as stash cannot be the controller of another account.
	// * an account already bonded as stash cannot nominate.
	// * an account already bonded as controller can nominate.
	with_externalities(&mut ExtBuilder::default()
		.sessions_per_era(2)
		.build(),
		|| {
			let arbitrary_value = 5;
			System::set_block_number(1);
			// 2 = controller, 1 stashed => ok
			assert_ok!(Staking::bond(Origin::signed(1), 2, arbitrary_value, RewardDestination::default()));
			// 2 = controller, 3 stashed (Note that 2 is reused.) => no-op
			assert_noop!(Staking::bond(Origin::signed(3), 2, arbitrary_value, RewardDestination::default()), "controller already paired");
		});
}

#[test]
fn session_and_eras_work() {
	with_externalities(&mut ExtBuilder::default()
		.sessions_per_era(2)
		.build(),
	|| {
		assert_eq!(Staking::era_length(), 2);
		assert_eq!(Staking::sessions_per_era(), 2);
		assert_eq!(Staking::last_era_length_change(), 0);
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 0);

		// Block 1: No change.
		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::current_index(), 1);
		assert_eq!(Staking::sessions_per_era(), 2);
		assert_eq!(Staking::last_era_length_change(), 0);
		assert_eq!(Staking::current_era(), 0);

		// Block 2: Simple era change.
		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::current_index(), 2);
		assert_eq!(Staking::sessions_per_era(), 2);
		assert_eq!(Staking::last_era_length_change(), 0);
		assert_eq!(Staking::current_era(), 1);

		// Block 3: Schedule an era length change; no visible changes.
		System::set_block_number(3);
		assert_ok!(Staking::set_sessions_per_era(3));
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::current_index(), 3);
		assert_eq!(Staking::sessions_per_era(), 2);
		assert_eq!(Staking::last_era_length_change(), 0);
		assert_eq!(Staking::current_era(), 1);

		// Block 4: Era change kicks in.
		System::set_block_number(4);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::current_index(), 4);
		assert_eq!(Staking::sessions_per_era(), 3);
		assert_eq!(Staking::last_era_length_change(), 4);
		assert_eq!(Staking::current_era(), 2);

		// Block 5: No change.
		System::set_block_number(5);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::current_index(), 5);
		assert_eq!(Staking::sessions_per_era(), 3);
		assert_eq!(Staking::last_era_length_change(), 4);
		assert_eq!(Staking::current_era(), 2);

		// Block 6: No change.
		System::set_block_number(6);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::current_index(), 6);
		assert_eq!(Staking::sessions_per_era(), 3);
		assert_eq!(Staking::last_era_length_change(), 4);
		assert_eq!(Staking::current_era(), 2);

		// Block 7: Era increment.
		System::set_block_number(7);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::current_index(), 7);
		assert_eq!(Staking::sessions_per_era(), 3);
		assert_eq!(Staking::last_era_length_change(), 4);
		assert_eq!(Staking::current_era(), 3);
	});
}

#[test]
fn cannot_transfer_staked_balance() {
	// Tests that a stash account cannot transfer funds
	with_externalities(&mut ExtBuilder::default().nominate(false).build(), || {
		// Confirm account 11 is stashed
		assert_eq!(Staking::bonded(&11), Some(10));
		// Confirm account 11 has some free balance
		assert_eq!(Balances::free_balance(&11), 1000);
		// Confirm account 11 (via controller 10) is totally staked
		assert_eq!(Staking::stakers(&11).total, 1000);
		// Confirm account 11 cannot transfer as a result
		assert_noop!(Balances::transfer(Origin::signed(11), 20, 1), "account liquidity restrictions prevent withdrawal");

		// Give account 11 extra free balance
		let _ = Balances::make_free_balance_be(&11, 10000);
		// Confirm that account 11 can now transfer some balance
		assert_ok!(Balances::transfer(Origin::signed(11), 20, 1));
	});
}

#[test]
fn cannot_transfer_staked_balance_2() {
	// Tests that a stash account cannot transfer funds
	// Same test as above but with 20
	// 21 has 2000 free balance but 1000 at stake
	with_externalities(&mut ExtBuilder::default()
		.nominate(false)
		.fare(true)
		.build(),
	|| {
		// Confirm account 21 is stashed
		assert_eq!(Staking::bonded(&21), Some(20));
		// Confirm account 21 has some free balance
		assert_eq!(Balances::free_balance(&21), 2000);
		// Confirm account 21 (via controller 20) is totally staked
		assert_eq!(Staking::stakers(&21).total, 1000);
		// Confirm account 21 can transfer at most 1000
		assert_noop!(Balances::transfer(Origin::signed(21), 20, 1001), "account liquidity restrictions prevent withdrawal");
		assert_ok!(Balances::transfer(Origin::signed(21), 20, 1000));
	});
}

#[test]
fn cannot_reserve_staked_balance() {
	// Checks that a bonded account cannot reserve balance from free balance
	with_externalities(&mut ExtBuilder::default().build(), || {
		// Confirm account 11 is stashed
		assert_eq!(Staking::bonded(&11), Some(10));
		// Confirm account 11 has some free balance
		assert_eq!(Balances::free_balance(&11), 1000);
		// Confirm account 11 (via controller 10) is totally staked
		assert_eq!(Staking::stakers(&11).total, 1125);
		// Confirm account 11 cannot transfer as a result
		assert_noop!(Balances::reserve(&11, 1), "account liquidity restrictions prevent withdrawal");

		// Give account 11 extra free balance
		let _ = Balances::make_free_balance_be(&11, 10000);
		// Confirm account 11 can now reserve balance
		assert_ok!(Balances::reserve(&11, 1));
	});
}

#[test]
fn reward_destination_works() {
	// Rewards go to the correct destination as determined in Payee
	with_externalities(&mut ExtBuilder::default().nominate(false).build(), || {
		// Check that account 11 is a validator
		assert!(<Validators<Test>>::exists(11));
		// Check that account 11 is a validator
		assert!(Staking::current_elected().contains(&11));
		// Check the balance of the validator account
		assert_eq!(Balances::free_balance(&10), 1);
		// Check the balance of the stash account
		assert_eq!(Balances::free_balance(&11), 1000);
		// Check these two accounts are bonded
		assert_eq!(Staking::bonded(&11), Some(10));
		// Check how much is at stake
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000, active: 1000, unlocking: vec![] }));
		// Track current session reward
		let mut current_session_reward = Staking::current_session_reward();

		// Move forward the system for payment
		System::set_block_number(1);
		Timestamp::set_timestamp(5);
		Session::check_rotate_session(System::block_number());

		// Check that RewardDestination is Staked (default)
		assert_eq!(Staking::payee(&11), RewardDestination::Staked);
		// Check current session reward is 10
		assert_eq!(current_session_reward, 10);
		// Check that reward went to the stash account of validator
		assert_eq!(Balances::free_balance(&11), 1000 + current_session_reward);
		// Check that amount at stake increased accordingly
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000 + 10, active: 1000 + 10, unlocking: vec![] }));
		// Update current session reward
		current_session_reward = Staking::current_session_reward(); // 1010 (1* slot_stake)

		//Change RewardDestination to Stash
		<Payee<Test>>::insert(&11, RewardDestination::Stash);

		// Move forward the system for payment
		System::set_block_number(2);
		Timestamp::set_timestamp(10);
		Session::check_rotate_session(System::block_number());

		// Check that RewardDestination is Stash
		assert_eq!(Staking::payee(&11), RewardDestination::Stash);
		// Check that reward went to the stash account
		assert_eq!(Balances::free_balance(&11), 1000 + 10 + current_session_reward);
		// Record this value
		let recorded_stash_balance = 1000 + 10 + current_session_reward;

		// Check that amount at stake is NOT increased
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000 + 10, active: 1000 + 10, unlocking: vec![] }));

		// Change RewardDestination to Controller
		<Payee<Test>>::insert(&11, RewardDestination::Controller);

		// Check controller balance
		assert_eq!(Balances::free_balance(&10), 1);

		// Move forward the system for payment
		System::set_block_number(3);
		Timestamp::set_timestamp(15);
		Session::check_rotate_session(System::block_number());

		// Check that RewardDestination is Controller
		assert_eq!(Staking::payee(&11), RewardDestination::Controller);
		// Check that reward went to the controller account
		assert_eq!(Balances::free_balance(&10), 1 + 1010);
		// Check that amount at stake is NOT increased
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000 + 10, active: 1000 + 10, unlocking: vec![] }));
		// Check that amount in staked account is NOT increased.
		assert_eq!(Balances::free_balance(&11), recorded_stash_balance);
	});
}

#[test]
fn validator_payment_prefs_work() {
	// Test that validator preferences are correctly honored
	// Note: unstake threshold is being directly tested in slashing tests.
	// This test will focus on validator payment.
	with_externalities(&mut ExtBuilder::default()
		.session_length(3)
		.sessions_per_era(3)
		.build(),
	|| {
		let session_reward = 10;
		let validator_cut = 5;
		let validator_initial_balance = Balances::total_balance(&11);
		// Initial config should be correct
		assert_eq!(Staking::era_length(), 9);
		assert_eq!(Staking::sessions_per_era(), 3);
		assert_eq!(Staking::last_era_length_change(), 0);
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 0);

		assert_eq!(Staking::current_session_reward(), session_reward);

		// check the balance of a validator accounts.
		assert_eq!(Balances::total_balance(&10), 1);
		// check the balance of a validator's stash accounts.
		assert_eq!(Balances::total_balance(&11), validator_initial_balance);
		// and the nominator (to-be)
		let _ = Balances::make_free_balance_be(&2, 500);

		// add a dummy nominator.
		// NOTE: this nominator is being added 'manually', use '.nominate()' to do it realistically.
		<Stakers<Test>>::insert(&11, Exposure {
			own: 500, // equal division indicates that the reward will be equally divided among validator and nominator.
			total: 1000,
			others: vec![IndividualExposure {who: 2, value: 500 }]
		});
		<Payee<Test>>::insert(&2, RewardDestination::Stash);
		<Validators<Test>>::insert(&11, ValidatorPrefs {
			unstake_threshold: 3,
			validator_payment: validator_cut
		});

		// ------------ Fast forward
		let mut block = 3;
		// Block 3 => Session 1 => Era 0
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5);	// on time.
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 1);

		// session triggered: the reward value stashed should be 10 -- defined in ExtBuilder genesis.
		assert_eq!(Staking::current_session_reward(), session_reward);
		assert_eq!(Staking::current_era_reward(), session_reward);

		block = 6; // Block 6 => Session 2 => Era 0
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5);	// a little late.
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 2);

		assert_eq!(Staking::current_session_reward(), session_reward);
		assert_eq!(Staking::current_era_reward(), 2*session_reward);

		block = 9; // Block 9 => Session 3 => Era 1
		System::set_block_number(block);
		Timestamp::set_timestamp(block*5);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 1);
		assert_eq!(Session::current_index(), 3);

		// whats left to be shared is the sum of 3 rounds minus the validator's cut.
		let shared_cut = 3 * session_reward - validator_cut;
		// Validator's payee is Staked account, 11, reward will be paid here.
		assert_eq!(Balances::total_balance(&11), validator_initial_balance + shared_cut/2 + validator_cut);
		// Controller account will not get any reward.
		assert_eq!(Balances::total_balance(&10), 1);
		// Rest of the reward will be shared and paid to the nominator in stake.
		assert_eq!(Balances::total_balance(&2), 500 + shared_cut/2);
	});

}

#[test]
fn bond_extra_works() {
	// Tests that extra `free_balance` in the stash can be added to stake
	// NOTE: this tests only verifies `StakingLedger` for correct updates
	// See `bond_extra_and_withdraw_unbonded_works` for more details and updates on `Exposure`.
	with_externalities(&mut ExtBuilder::default().build(),
	|| {
		// Check that account 10 is a validator
		assert!(<Validators<Test>>::exists(11));
		// Check that account 10 is bonded to account 11
		assert_eq!(Staking::bonded(&11), Some(10));
		// Check how much is at stake
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000, active: 1000, unlocking: vec![] }));

		// Give account 11 some large free balance greater than total
		let _ = Balances::make_free_balance_be(&11, 1000000);

		// Call the bond_extra function from controller, add only 100
		assert_ok!(Staking::bond_extra(Origin::signed(11), 100));
		// There should be 100 more `total` and `active` in the ledger
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000 + 100, active: 1000 + 100, unlocking: vec![] }));

		// Call the bond_extra function with a large number, should handle it
		assert_ok!(Staking::bond_extra(Origin::signed(11), u64::max_value()));
		// The full amount of the funds should now be in the total and active
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000000, active: 1000000, unlocking: vec![] }));

	});
}

#[test]
fn bond_extra_and_withdraw_unbonded_works() {
	// * Should test
	// * Given an account being bonded [and chosen as a validator](not mandatory)
	// * It can add extra funds to the bonded account.
	// * it can unbond a portion of its funds from the stash account.
	// * Once the unbonding period is done, it can actually take the funds out of the stash.
	with_externalities(&mut ExtBuilder::default()
		.nominate(false)
		.build(),
	|| {
		// Set payee to controller. avoids confusion
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Controller));

		// Set unbonding era (bonding_duration) to 2
		assert_ok!(Staking::set_bonding_duration(2));

		// Give account 11 some large free balance greater than total
		let _ = Balances::make_free_balance_be(&11, 1000000);

		// Initial config should be correct
		assert_eq!(Staking::sessions_per_era(), 1);
		assert_eq!(Staking::current_era(), 0);
		assert_eq!(Session::current_index(), 0);
		assert_eq!(Staking::current_session_reward(), 10);

		// check the balance of a validator accounts.
		assert_eq!(Balances::total_balance(&10), 1);

		// confirm that 10 is a normal validator and gets paid at the end of the era.
		System::set_block_number(1);
		Timestamp::set_timestamp(5);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 1);
		assert_eq!(Session::current_index(), 1);

		// NOTE: despite having .nominate() in extBuilder, 20 doesn't have a share since
		// rewards are paid before election in new_era()
		assert_eq!(Balances::total_balance(&10), 1 + 10);

		// Initial state of 10
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000, active: 1000, unlocking: vec![] }));
		assert_eq!(Staking::stakers(&11), Exposure { total: 1000, own: 1000, others: vec![] });

		// deposit the extra 100 units
		Staking::bond_extra(Origin::signed(11), 100).unwrap();

		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000 + 100, active: 1000 + 100, unlocking: vec![] }));
		// Exposure is a snapshot! only updated after the next era update.
		assert_ne!(Staking::stakers(&11), Exposure { total: 1000 + 100, own: 1000 + 100, others: vec![] });

		// trigger next era.
		System::set_block_number(2);Timestamp::set_timestamp(10);Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 2);
		assert_eq!(Session::current_index(), 2);

		// ledger should be the same.
		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000 + 100, active: 1000 + 100, unlocking: vec![] }));
		// Exposure is now updated.
		assert_eq!(Staking::stakers(&11), Exposure { total: 1000 + 100, own: 1000 + 100, others: vec![] });
		// Note that by this point 10 also have received more rewards, but we don't care now.
		// assert_eq!(Balances::total_balance(&10), 1 + 10 + MORE_REWARD);

		// Unbond almost all of the funds in stash.
		Staking::unbond(Origin::signed(10), 1000).unwrap();
		assert_eq!(Staking::ledger(&10), Some(StakingLedger {
			stash: 11, total: 1000 + 100, active: 100, unlocking: vec![UnlockChunk{ value: 1000, era: 2 + 2}] }));

		// Attempting to free the balances now will fail. 2 eras need to pass.
		Staking::withdraw_unbonded(Origin::signed(10)).unwrap();
		assert_eq!(Staking::ledger(&10), Some(StakingLedger {
			stash: 11, total: 1000 + 100, active: 100, unlocking: vec![UnlockChunk{ value: 1000, era: 2 + 2}] }));

		// trigger next era.
		System::set_block_number(3);Timestamp::set_timestamp(15);Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 3);
		assert_eq!(Session::current_index(), 3);

		// nothing yet
		Staking::withdraw_unbonded(Origin::signed(10)).unwrap();
		assert_eq!(Staking::ledger(&10), Some(StakingLedger {
			stash: 11, total: 1000 + 100, active: 100, unlocking: vec![UnlockChunk{ value: 1000, era: 2 + 2}] }));

		// trigger next era.
		System::set_block_number(4);Timestamp::set_timestamp(20);Session::check_rotate_session(System::block_number());
		assert_eq!(Staking::current_era(), 4);
		assert_eq!(Session::current_index(), 4);

		Staking::withdraw_unbonded(Origin::signed(10)).unwrap();
		// Now the value is free and the staking ledger is updated.
		assert_eq!(Staking::ledger(&10), Some(StakingLedger {
			stash: 11, total: 100, active: 100, unlocking: vec![] }));
	})
}

#[test]
fn slot_stake_is_least_staked_validator_and_limits_maximum_punishment() {
	// Test that slot_stake is determined by the least staked validator
	// Test that slot_stake is the maximum punishment that can happen to a validator
	// Note that rewardDestination is the stash account by default
	// Note that unlike reward slash will affect free_balance, not the stash account.
	with_externalities(&mut ExtBuilder::default()
		.nominate(false)
		.fare(false)
		.build(),
	|| {
		// Give the man some money.
		// Confirm validator count is 2
		assert_eq!(Staking::validator_count(), 2);
		// Confirm account 10 and 20 are validators
		assert!(<Validators<Test>>::exists(&11) && <Validators<Test>>::exists(&21));

		assert_eq!(Staking::stakers(&11).total, 1000);
		assert_eq!(Staking::stakers(&21).total, 2000);

		// Give the man some money.
		let _ = Balances::make_free_balance_be(&10, 1000);
		let _ = Balances::make_free_balance_be(&20, 1000);

		// We confirm initialized slot_stake is this value
		assert_eq!(Staking::slot_stake(), Staking::stakers(&11).total);

		// Now lets lower account 20 stake
		<Stakers<Test>>::insert(&21, Exposure { total: 69, own: 69, others: vec![] });
		assert_eq!(Staking::stakers(&21).total, 69);
		<Ledger<Test>>::insert(&20, StakingLedger { stash: 22, total: 69, active: 69, unlocking: vec![] });

		// New era --> rewards are paid --> stakes are changed
		System::set_block_number(1);
		Timestamp::set_timestamp(5);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Staking::current_era(), 1);
		// -- new balances + reward
		assert_eq!(Staking::stakers(&11).total, 1000 + 10);
		assert_eq!(Staking::stakers(&21).total, 69 + 10);

		// -- Note that rewards are going directly to stash, not as free balance.
		assert_eq!(Balances::free_balance(&10), 1000);
		assert_eq!(Balances::free_balance(&20), 1000);

		// -- slot stake should also be updated.
		assert_eq!(Staking::slot_stake(), 79);

		// // If 10 gets slashed now, despite having +1000 in stash, it will be slashed byt 79, which is the slot stake
		Staking::on_offline_validator(10, 4);
		// // Confirm user has been reported
		assert_eq!(Staking::slash_count(&11), 4);
		// // check the balance of 10 (slash will be deducted from free balance.)
		assert_eq!(Balances::free_balance(&11), 1000 + 10 - 50 /*5% of 1000*/ * 8 /*2**3*/);
	});
}

#[test]
fn on_free_balance_zero_stash_removes_validator() {
	// Tests that validator storage items are cleaned up when stash is empty
	// Tests that storage items are untouched when controller is empty
	with_externalities(&mut ExtBuilder::default()
		.existential_deposit(10)
		.build(),
	|| {
		// Check that account 10 is a validator
		assert!(<Validators<Test>>::exists(11));
		// Check the balance of the validator account
		assert_eq!(Balances::free_balance(&10), 256);
		// Check the balance of the stash account
		assert_eq!(Balances::free_balance(&11), 256000);
		// Check these two accounts are bonded
		assert_eq!(Staking::bonded(&11), Some(10));

		// Set some storage items which we expect to be cleaned up
		// Initiate slash count storage item
		Staking::on_offline_validator(10, 1);
		// Set payee information
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Stash));

		// Check storage items that should be cleaned up
		assert!(<Ledger<Test>>::exists(&10));
		assert!(<Bonded<Test>>::exists(&11));
		assert!(<Validators<Test>>::exists(&11));
		assert!(<SlashCount<Test>>::exists(&11));
		assert!(<Payee<Test>>::exists(&11));

		// Reduce free_balance of controller to 0
		Balances::slash(&10, u64::max_value());

		// Check the balance of the stash account has not been touched
		assert_eq!(Balances::free_balance(&11), 256000);
		// Check these two accounts are still bonded
		assert_eq!(Staking::bonded(&11), Some(10));

		// Check storage items have not changed
		assert!(<Ledger<Test>>::exists(&10));
		assert!(<Bonded<Test>>::exists(&11));
		assert!(<Validators<Test>>::exists(&11));
		assert!(<SlashCount<Test>>::exists(&11));
		assert!(<Payee<Test>>::exists(&11));

		// Reduce free_balance of stash to 0
		Balances::slash(&11, u64::max_value());
		// Check total balance of stash
		assert_eq!(Balances::total_balance(&11), 0);

		// Check storage items do not exist
		assert!(!<Ledger<Test>>::exists(&10));
		assert!(!<Bonded<Test>>::exists(&11));
		assert!(!<Validators<Test>>::exists(&11));
		assert!(!<Nominators<Test>>::exists(&11));
		assert!(!<SlashCount<Test>>::exists(&11));
		assert!(!<Payee<Test>>::exists(&11));
	});
}

#[test]
fn on_free_balance_zero_stash_removes_nominator() {
	// Tests that nominator storage items are cleaned up when stash is empty
	// Tests that storage items are untouched when controller is empty
	with_externalities(&mut ExtBuilder::default()
		.existential_deposit(10)
		.build(),
	|| {
		// Make 10 a nominator
		assert_ok!(Staking::nominate(Origin::signed(10), vec![20]));
		// Check that account 10 is a nominator
		assert!(<Nominators<Test>>::exists(11));
		// Check the balance of the nominator account
		assert_eq!(Balances::free_balance(&10), 256);
		// Check the balance of the stash account
		assert_eq!(Balances::free_balance(&11), 256000);
		// Check these two accounts are bonded
		assert_eq!(Staking::bonded(&11), Some(10));

		// Set payee information
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Stash));


		// Check storage items that should be cleaned up
		assert!(<Ledger<Test>>::exists(&10));
		assert!(<Bonded<Test>>::exists(&11));
		assert!(<Nominators<Test>>::exists(&11));
		assert!(<Payee<Test>>::exists(&11));

		// Reduce free_balance of controller to 0
		Balances::slash(&10, u64::max_value());
		// Check total balance of account 10
		assert_eq!(Balances::total_balance(&10), 0);

		// Check the balance of the stash account has not been touched
		assert_eq!(Balances::free_balance(&11), 256000);
		// Check these two accounts are still bonded
		assert_eq!(Staking::bonded(&11), Some(10));

		// Check storage items have not changed
		assert!(<Ledger<Test>>::exists(&10));
		assert!(<Bonded<Test>>::exists(&11));
		assert!(<Nominators<Test>>::exists(&11));
		assert!(<Payee<Test>>::exists(&11));

		// Reduce free_balance of stash to 0
		Balances::slash(&11, u64::max_value());
		// Check total balance of stash
		assert_eq!(Balances::total_balance(&11), 0);

		// Check storage items do not exist
		assert!(!<Ledger<Test>>::exists(&10));
		assert!(!<Bonded<Test>>::exists(&11));
		assert!(!<Validators<Test>>::exists(&11));
		assert!(!<Nominators<Test>>::exists(&11));
		assert!(!<SlashCount<Test>>::exists(&11));
		assert!(!<Payee<Test>>::exists(&11));
	});
}

#[test]
fn phragmen_poc_works() {
	// Tests the POC test of the phragmen, mentioned in the paper and reference implementation.
	// Initial votes:
	// Votes  [
	// ('2', 500, ['10', '20', '30']),
	// ('4', 500, ['10', '20', '40']),
	// ('10', 1000, ['10']),
	// ('20', 1000, ['20']),
	// ('30', 1000, ['30']),
	// ('40', 1000, ['40'])]
	//
	// Sequential Phragmén gives
	// 10  is elected with stake  1666.6666666666665 and score  0.0005
	// 20  is elected with stake  1333.3333333333333 and score  0.00075

	// 2  has load  0.00075 and supported
	// 10  with stake  333.3333333333333 20  with stake  166.66666666666666 30  with stake  0.0
	// 4  has load  0.00075 and supported
	// 10  with stake  333.3333333333333 20  with stake  166.66666666666666 40  with stake  0.0
	// 10  has load  0.0005 and supported
	// 10  with stake  1000.0
	// 20  has load  0.00075 and supported
	// 20  with stake  1000.0
	// 30  has load  0 and supported
	// 30  with stake  0
	// 40  has load  0 and supported
	// 40  with stake  0

	// 	Sequential Phragmén with post processing gives
	// 10  is elected with stake  1500.0 and score  0.0005
	// 20  is elected with stake  1500.0 and score  0.00075
	//
	// 10  has load  0.0005 and supported
	// 10  with stake  1000.0
	// 20  has load  0.00075 and supported
	// 20  with stake  1000.0
	// 30  has load  0 and supported
	// 30  with stake  0
	// 40  has load  0 and supported
	// 40  with stake  0
	// 2  has load  0.00075 and supported
	// 10  with stake  166.66666666666674 20  with stake  333.33333333333326 30  with stake  0
	// 4  has load  0.00075 and supported
	// 10  with stake  333.3333333333333 20  with stake  166.66666666666666 40  with stake  0.0


	with_externalities(&mut ExtBuilder::default()
		.nominate(false)
		.validator_pool(true)
		.build(),
	|| {
		// We don't really care about this. At this point everything is even.
		// assert_eq!(Session::validators(), vec![40, 30]);

		assert_eq!(Staking::ledger(&10), Some(StakingLedger { stash: 11, total: 1000, active: 1000, unlocking: vec![] }));
		assert_eq!(Staking::ledger(&20), Some(StakingLedger { stash: 21, total: 1000, active: 1000, unlocking: vec![] }));
		assert_eq!(Staking::ledger(&30), Some(StakingLedger { stash: 31, total: 1000, active: 1000, unlocking: vec![] }));
		assert_eq!(Staking::ledger(&40), Some(StakingLedger { stash: 41, total: 1000, active: 1000, unlocking: vec![] }));

		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Controller));
		assert_ok!(Staking::set_payee(Origin::signed(20), RewardDestination::Controller));
		assert_ok!(Staking::set_payee(Origin::signed(30), RewardDestination::Controller));
		assert_ok!(Staking::set_payee(Origin::signed(40), RewardDestination::Controller));

		// no one is a nominator
		assert_eq!(<Nominators<Test>>::enumerate().count(), 0 as usize);

		// bond [2,1] / [4,3] a nominator
		let _ = Balances::deposit_creating(&1, 1000);
		let _ = Balances::deposit_creating(&3, 1000);

		assert_ok!(Staking::bond(Origin::signed(1), 2, 500, RewardDestination::default()));
		assert_ok!(Staking::nominate(Origin::signed(2), vec![11, 21, 31]));

		assert_ok!(Staking::bond(Origin::signed(3), 4, 500, RewardDestination::default()));
		assert_ok!(Staking::nominate(Origin::signed(4), vec![11, 21, 41]));

		// New era => election algorithm will trigger
		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators(), vec![20, 10]);

		// with stake 1666 and 1333 respectively
		assert_eq!(Staking::stakers(11).own, 1000);
		assert_eq!(Staking::stakers(11).total, 1000 + 332);
		assert_eq!(Staking::stakers(21).own, 1000);
		assert_eq!(Staking::stakers(21).total, 1000 + 666);

		// Nominator's stake distribution.
		assert_eq!(Staking::stakers(11).others.iter().map(|e| e.value).collect::<Vec<BalanceOf<Test>>>(), vec![166, 166]);
		assert_eq!(Staking::stakers(11).others.iter().map(|e| e.value).sum::<BalanceOf<Test>>(), 332);
		assert_eq!(Staking::stakers(11).others.iter().map(|e| e.who).collect::<Vec<BalanceOf<Test>>>(), vec![3, 1]);

		assert_eq!(Staking::stakers(21).others.iter().map(|e| e.value).collect::<Vec<BalanceOf<Test>>>(), vec![333, 333]);
		assert_eq!(Staking::stakers(21).others.iter().map(|e| e.value).sum::<BalanceOf<Test>>(), 666);
		assert_eq!(Staking::stakers(21).others.iter().map(|e| e.who).collect::<Vec<BalanceOf<Test>>>(), vec![3, 1]);
	});
}

#[test]
fn phragmen_election_works_example_2() {
	// tests the encapsulated phragmen::elect function.
	with_externalities(&mut ExtBuilder::default().nominate(false).build(), || {
		// initial setup of 10 and 20, both validators
		assert_eq!(Session::validators(), vec![20, 10]);

		// no one is a nominator
		assert_eq!(<Nominators<Test>>::enumerate().count(), 0 as usize);

		// Bond [30, 31] as the third validator
		assert_ok!(Staking::bond(Origin::signed(31), 30, 1000, RewardDestination::default()));
		assert_ok!(Staking::validate(Origin::signed(30), ValidatorPrefs::default()));

		// bond [2,1](A), [4,3](B), as 2 nominators
		// Give all of them some balance to be able to bond properly.
		for i in &[1, 3] { let _ = Balances::deposit_creating(i, 2000); }
		assert_ok!(Staking::bond(Origin::signed(1), 2, 50, RewardDestination::default()));
		assert_ok!(Staking::nominate(Origin::signed(2), vec![11, 21]));

		assert_ok!(Staking::bond(Origin::signed(3), 4, 1000, RewardDestination::default()));
		assert_ok!(Staking::nominate(Origin::signed(4), vec![11, 31]));

		let rounds =     || 2 as usize;
		let validators = || <Validators<Test>>::enumerate();
		let nominators = || <Nominators<Test>>::enumerate();
		let min_validator_count = Staking::minimum_validator_count() as usize;

		let winners = phragmen::elect::<Test, _, _, _, _>(
			rounds,
			validators,
			nominators,
			Staking::slashable_balance_of,
			min_validator_count,
			ElectionConfig::<BalanceOf<Test>> {
				equalise: true,
				tolerance: <BalanceOf<Test>>::sa(10 as u64),
				iterations: 10,
			}
		);

		let winners = winners.unwrap();

		// 10 and 30 must be the winners
		assert_eq!(winners.iter().map(|w| w.who).collect::<Vec<BalanceOf<Test>>>(), vec![11, 31]);

		let winner_10 = winners.iter().filter(|w| w.who == 11).nth(0).unwrap();
		let winner_30 = winners.iter().filter(|w| w.who == 31).nth(0).unwrap();

		// python implementation output:
		/*
		Votes  [
			('10', 1000, ['10']),
			('20', 1000, ['20']),
			('30', 1000, ['30']),
			('2', 50, ['10', '20']),
			('4', 1000, ['10', '30'])
		]
		Sequential Phragmén gives
		10  is elected with stake  1705.7377049180327 and score  0.0004878048780487805
		30  is elected with stake  1344.2622950819673 and score  0.0007439024390243903

		10  has load  0.0004878048780487805 and supported
		10  with stake  1000.0
		20  has load  0 and supported
		20  with stake  0
		30  has load  0.0007439024390243903 and supported
		30  with stake  1000.0
		2  has load  0.0004878048780487805 and supported
		10  with stake  50.0 20  with stake  0.0
		4  has load  0.0007439024390243903 and supported
		10  with stake  655.7377049180328 30  with stake  344.26229508196724

		Sequential Phragmén with post processing gives
		10  is elected with stake  1525.0 and score  0.0004878048780487805
		30  is elected with stake  1525.0 and score  0.0007439024390243903

		10  has load  0.0004878048780487805 and supported
		10  with stake  1000.0
		20  has load  0 and supported
		20  with stake  0
		30  has load  0.0007439024390243903 and supported
		30  with stake  1000.0
		2  has load  0.0004878048780487805 and supported
		10  with stake  50.0 20  with stake  0.0
		4  has load  0.0007439024390243903 and supported
		10  with stake  475.0 30  with stake  525.0


		*/

		assert_eq!(winner_10.exposure.total, 1000 + 525);
		assert_eq!(winner_10.score, Perquintill::from_quintillionths(487804878048780));
		assert_eq!(winner_10.exposure.others[0].value, 475);
		assert_eq!(winner_10.exposure.others[1].value, 50);

		assert_eq!(winner_30.exposure.total, 1000 + 525);
		assert_eq!(winner_30.score, Perquintill::from_quintillionths(743902439024390));
		assert_eq!(winner_30.exposure.others[0].value, 525);
	})
}

#[test]
fn switching_roles() {
	// Show: It should be possible to switch between roles (nominator, validator, idle) with minimal overhead.
	with_externalities(&mut ExtBuilder::default()
		.nominate(false)
		.sessions_per_era(3)
		.build(),
	|| {
		// Reset reward destination
		for i in &[10, 20] { assert_ok!(Staking::set_payee(Origin::signed(*i), RewardDestination::Controller)); }

		assert_eq!(Session::validators(), vec![20, 10]);

		// put some money in account that we'll use.
		for i in 1..7 { let _ = Balances::deposit_creating(&i, 5000); }

		// add 2 nominators
		assert_ok!(Staking::bond(Origin::signed(1), 2, 2000, RewardDestination::Controller));
		assert_ok!(Staking::nominate(Origin::signed(2), vec![11, 5]));

		assert_ok!(Staking::bond(Origin::signed(3), 4, 500, RewardDestination::Controller));
		assert_ok!(Staking::nominate(Origin::signed(4), vec![21, 1]));

		// add a new validator candidate
		assert_ok!(Staking::bond(Origin::signed(5), 6, 1000, RewardDestination::Controller));
		assert_ok!(Staking::validate(Origin::signed(6), ValidatorPrefs::default()));

		// new block
		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		// no change
		assert_eq!(Session::validators(), vec![20, 10]);

		// new block
		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());

		// no change
		assert_eq!(Session::validators(), vec![20, 10]);

		// new block --> ne era --> new validators
		System::set_block_number(3);
		Session::check_rotate_session(System::block_number());

		// with current nominators 10 and 5 have the most stake
		assert_eq!(Session::validators(), vec![6, 10]);

		// 2 decides to be a validator. Consequences:
		// new stakes:
		// 10: 1000 self vote
		// 6: 1000 self vote
		// 20: 1000 self vote + 500 vote
		// 2: 2000 self  vote + 500 vote.
		assert_ok!(Staking::validate(Origin::signed(2), ValidatorPrefs::default()));

		System::set_block_number(4);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::validators(), vec![6, 10]);

		System::set_block_number(5);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::validators(), vec![6, 10]);

		// ne era
		System::set_block_number(6);
		Session::check_rotate_session(System::block_number());
		assert_eq!(Session::validators(), vec![2, 20]);
	});
}

#[test]
fn wrong_vote_is_null() {
	with_externalities(&mut ExtBuilder::default()
		.nominate(false)
		.validator_pool(true)
	.build(),
	|| {
		assert_eq!(Session::validators(), vec![40, 30]);

		// put some money in account that we'll use.
		for i in 1..3 { let _ = Balances::deposit_creating(&i, 5000); }

		// add 1 nominators
		assert_ok!(Staking::bond(Origin::signed(1), 2, 2000, RewardDestination::default()));
		assert_ok!(Staking::nominate(Origin::signed(2), vec![
			11, 21, 			// good votes
			1, 2, 15, 1000, 25  // crap votes. No effect.
		]));

		// new block
		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators(), vec![20, 10]);
	});
}

#[test]
fn bond_with_no_staked_value() {
	// Behavior when someone bonds with no staked value.
	// Particularly when she votes and the candidate is elected.
	with_externalities(&mut ExtBuilder::default()
	.validator_count(3)
	.nominate(false)
	.minimum_validator_count(1)
	.build(), || {
		// setup
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Controller));
		assert_ok!(Staking::set_payee(Origin::signed(20), RewardDestination::Controller));
		let _ = Balances::deposit_creating(&3, 1000);
		let initial_balance_2 = Balances::free_balance(&2);
		let initial_balance_4 = Balances::free_balance(&4);

		// initial validators
		assert_eq!(Session::validators(), vec![20, 10]);

		// Stingy validator.
		assert_ok!(Staking::bond(Origin::signed(1), 2, 0, RewardDestination::Controller));
		assert_ok!(Staking::validate(Origin::signed(2), ValidatorPrefs::default()));

		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		// Not elected even though we want 3.
		assert_eq!(Session::validators(), vec![20, 10]);

		// min of 10 and 20.
		assert_eq!(Staking::slot_stake(), 1000);

		// let's make the stingy one elected.
		assert_ok!(Staking::bond(Origin::signed(3), 4, 500, RewardDestination::Controller));
		assert_ok!(Staking::nominate(Origin::signed(4), vec![1]));

		assert_eq!(Staking::ledger(4), Some(StakingLedger { stash: 3, active: 500, total: 500, unlocking: vec![]}));

		assert_eq!(Balances::free_balance(&2), initial_balance_2);
		assert_eq!(Balances::free_balance(&4), initial_balance_4);

		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators(), vec![20, 10, 2]);
		assert_eq!(Staking::stakers(1), Exposure { own: 0, total: 500, others: vec![IndividualExposure { who: 3, value: 500}]});

		assert_eq!(Staking::slot_stake(), 500);

		// no rewards paid to 2 and 4 yet
		assert_eq!(Balances::free_balance(&2), initial_balance_2);
		assert_eq!(Balances::free_balance(&4), initial_balance_4);

		System::set_block_number(3);
		Session::check_rotate_session(System::block_number());

		let reward = Staking::current_session_reward();
		// 2 will not get any reward
		// 4 will get all the reward share
		assert_eq!(Balances::free_balance(&2), initial_balance_2);
		// assert_eq!(Balances::free_balance(&4), initial_balance_4 + reward);
		assert_eq!(Balances::free_balance(&4), initial_balance_4 + reward);
	});
}

#[test]
fn bond_with_little_staked_value() {
	// Behavior when someone bonds with little staked value.
	// Particularly when she votes and the candidate is elected.
	with_externalities(&mut ExtBuilder::default()
		.validator_count(3)
		.nominate(false)
		.minimum_validator_count(1)
		.build(),
	|| {
		// setup
		assert_ok!(Staking::set_payee(Origin::signed(10), RewardDestination::Controller));
		assert_ok!(Staking::set_payee(Origin::signed(20), RewardDestination::Controller));
		let initial_balance_2 = Balances::free_balance(&2);

		// initial validators
		assert_eq!(Session::validators(), vec![20, 10]);

		// Stingy validator.
		assert_ok!(Staking::bond(Origin::signed(1), 2, 1, RewardDestination::Controller));
		assert_ok!(Staking::validate(Origin::signed(2), ValidatorPrefs::default()));

		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		// 2 is elected.
		// and fucks up the slot stake.
		assert_eq!(Session::validators(), vec![20, 10, 2]);
		assert_eq!(Staking::slot_stake(), 1);

		// Old ones are rewarded.
		assert_eq!(Balances::free_balance(&10), 1 + 10);
		assert_eq!(Balances::free_balance(&20), 1 + 10);
		// no rewards paid to 2. This was initial election.
		assert_eq!(Balances::free_balance(&2), initial_balance_2);

		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators(), vec![20, 10, 2]);
		assert_eq!(Staking::slot_stake(), 1);

		let reward = Staking::current_session_reward();
		// 2 will not get the full reward, practically 1
		assert_eq!(Balances::free_balance(&2), initial_balance_2 + reward.max(1));
	});
}


#[test]
#[ignore] // Enable this once post-processing is on.
fn phragmen_linear_worse_case_equalise() {
	with_externalities(&mut ExtBuilder::default()
		.nominate(false)
		.validator_pool(true)
		.fare(true)
		.build(),
	|| {
		let bond_validator = |a, b| {
			let _ = Balances::deposit_creating(&(a-1), b);
			assert_ok!(Staking::bond(Origin::signed(a-1), a, b, RewardDestination::Controller));
			assert_ok!(Staking::validate(Origin::signed(a), ValidatorPrefs::default()));
		};
		let bond_nominator = |a, b, v| {
			let _ = Balances::deposit_creating(&(a-1), b);
			assert_ok!(Staking::bond(Origin::signed(a-1), a, b, RewardDestination::Controller));
			assert_ok!(Staking::nominate(Origin::signed(a), v));
		};

		for i in &[10, 20, 30, 40] { assert_ok!(Staking::set_payee(Origin::signed(*i), RewardDestination::Controller)); }

		bond_validator(50, 1000);
		bond_validator(60, 1000);
		bond_validator(70, 1000);

		bond_nominator(2, 2000, vec![11]);
		bond_nominator(4, 1000, vec![11, 21]);
		bond_nominator(6, 1000, vec![21, 31]);
		bond_nominator(8, 1000, vec![31, 41]);
		bond_nominator(110, 1000, vec![41, 51]);
		bond_nominator(112, 1000, vec![51, 61]);
		bond_nominator(114, 1000, vec![61, 71]);

		assert_eq!(Session::validators(), vec![40, 30]);
		assert_ok!(Staking::set_validator_count(7));

		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators(), vec![10, 60, 40, 20, 50, 30, 70]);

		// Sequential Phragmén with post processing gives
		// 10  is elected with stake  3000.0 and score  0.00025
		// 30  is elected with stake  2008.8712884829595 and score  0.0003333333333333333
		// 50  is elected with stake  2000.0001049958742 and score  0.0003333333333333333
		// 60  is elected with stake  1991.128921508789 and score  0.0004444444444444444
		// 20  is elected with stake  2017.7421569824219 and score  0.0005277777777777777
		// 40  is elected with stake  2000.0001049958742 and score  0.0005555555555555556
		// 70  is elected with stake  1982.2574230340813 and score  0.0007222222222222222

		assert_eq!(Staking::stakers(11).total, 3000);
		assert_eq!(Staking::stakers(31).total, 2035);
		assert_eq!(Staking::stakers(51).total, 2000);
		assert_eq!(Staking::stakers(61).total, 1968);
		assert_eq!(Staking::stakers(21).total, 2035);
		assert_eq!(Staking::stakers(41).total, 2024);
		assert_eq!(Staking::stakers(71).total, 1936);
	})
}

#[test]
fn phragmen_chooses_correct_validators() {
	with_externalities(&mut ExtBuilder::default()
		.nominate(true)
		.validator_pool(true)
		.fare(true)
		.validator_count(1)
		.build(),
	|| {
		// 4 validator candidates
		// self vote + default account 100 is nominator.
		assert_eq!(Staking::validator_count(), 1);
		assert_eq!(Session::validators().len(), 1);

		System::set_block_number(1);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators().len(), 1);
	})
}

#[test]
fn phragmen_should_not_overflow_validators() {
	with_externalities(&mut ExtBuilder::default()
	.nominate(false)
	.build()
	, || {
		let bond_validator = |a, b| {
			assert_ok!(Staking::bond(Origin::signed(a-1), a, b, RewardDestination::Controller));
			assert_ok!(Staking::validate(Origin::signed(a), ValidatorPrefs::default()));
		};
		let bond_nominator = |a, b, v| {
			assert_ok!(Staking::bond(Origin::signed(a-1), a, b, RewardDestination::Controller));
			assert_ok!(Staking::nominate(Origin::signed(a), v));
		};

		for i in 1..=8 {
			let _ = Balances::make_free_balance_be(&i, u64::max_value());
		}

		let _ = Staking::chill(Origin::signed(10));
		let _ = Staking::chill(Origin::signed(20));

		bond_validator(2, u64::max_value());
		bond_validator(4, u64::max_value());

		bond_nominator(6, u64::max_value()/2, vec![1, 3]);
		bond_nominator(8, u64::max_value()/2, vec![1, 3]);

		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators(), vec![4, 2]);
	})
}

#[test]
fn phragmen_should_not_overflow_nominators() {
	with_externalities(&mut ExtBuilder::default()
	.nominate(false)
	.build()
	, || {
		let bond_validator = |a, b| {
			assert_ok!(Staking::bond(Origin::signed(a-1), a, b, RewardDestination::Controller));
			assert_ok!(Staking::validate(Origin::signed(a), ValidatorPrefs::default()));
		};
		let bond_nominator = |a, b, v| {
			assert_ok!(Staking::bond(Origin::signed(a-1), a, b, RewardDestination::Controller));
			assert_ok!(Staking::nominate(Origin::signed(a), v));
		};

		let _ = Staking::chill(Origin::signed(10));
		let _ = Staking::chill(Origin::signed(20));

		for i in 1..=8 {
			let _ = Balances::make_free_balance_be(&i, u64::max_value());
		}

		bond_validator(2, u64::max_value()/2);
		bond_validator(4, u64::max_value()/2);

		bond_nominator(6, u64::max_value(), vec![1, 3]);
		bond_nominator(8, u64::max_value(), vec![1, 3]);

		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators(), vec![4, 2]);
	})
}

#[test]
fn phragmen_should_not_overflow_ultimate() {
	with_externalities(&mut ExtBuilder::default()
	.nominate(false)
	.build()
	, || {
		let bond_validator = |a, b| {
			assert_ok!(Staking::bond(Origin::signed(a-1), a, b, RewardDestination::Controller));
			assert_ok!(Staking::validate(Origin::signed(a), ValidatorPrefs::default()));
		};
		let bond_nominator = |a, b, v| {
			assert_ok!(Staking::bond(Origin::signed(a-1), a, b, RewardDestination::Controller));
			assert_ok!(Staking::nominate(Origin::signed(a), v));
		};

		for i in 1..=8 {
			let _ = Balances::make_free_balance_be(&i, u64::max_value());
		}

		bond_validator(2, u64::max_value());
		bond_validator(4, u64::max_value());

		bond_nominator(6, u64::max_value(), vec![1, 3]);
		bond_nominator(8, u64::max_value(), vec![1, 3]);

		System::set_block_number(2);
		Session::check_rotate_session(System::block_number());

		assert_eq!(Session::validators(), vec![4, 2]);
	})
}