import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import verifier


class IndependentVerifierTests(unittest.TestCase):
    def test_valid_representations(self):
        self.assertTrue(verifier.validate_representation(31, 17, 7))
        self.assertTrue(verifier.validate_representation(32, 29, 3))

    def test_invalid_p(self):
        self.assertFalse(verifier.validate_representation(31, 19, 7))

    def test_invalid_q(self):
        self.assertFalse(verifier.validate_representation(31, 17, 13))

    def test_invalid_six_q_minus_one(self):
        self.assertFalse(verifier.validate_representation(31, 17, 13))

    def test_invalid_six_q_plus_one(self):
        self.assertFalse(verifier.validate_representation(31, 17, 19))

    def test_wrong_parity_equation(self):
        self.assertFalse(verifier.validate_representation(32, 17, 7))

    def test_p_must_exceed_q(self):
        self.assertFalse(verifier.validate_representation(14, 7, 7))

    def test_known_zero_solution(self):
        self.assertEqual(verifier.verify_n(91)["status"], 0)

    def test_domain_boundaries(self):
        self.assertFalse(verifier.in_conjecture_domain(8068))
        self.assertTrue(verifier.in_conjecture_domain(8070))
        self.assertFalse(verifier.in_conjecture_domain(18679))
        self.assertTrue(verifier.in_conjecture_domain(18681))
        self.assertEqual(verifier.ODD_CONJECTURE_MIN, 18680)

    def test_record_validation(self):
        self.assertTrue(verifier.verify_record(verifier.verify_n(31)))
        self.assertTrue(verifier.verify_record(verifier.verify_n(91)))


if __name__ == "__main__":
    unittest.main()
