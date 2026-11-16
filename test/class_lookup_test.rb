# frozen_string_literal: true

require_relative 'test_helper'
require 'open3'
require 'rbconfig'

# The class of the value the extension wraps (Ledger) is fixed when it loads.
# Nothing a caller does later, redefining the constant or `Object.const_get`,
# reaches a ledger being built. Each case runs in a fresh process: the class
# is fixed once per process, at load.
class ClassLookupTest < Minitest::Test
  LIB = File.expand_path('../lib', __dir__)

  PRELUDE = <<~RUBY
    require 'dedup_machines'
    D = DedupMachines
  RUBY

  # Runs `body` after PRELUDE in a new Ruby; answers its last line of output.
  def isolated(body)
    out, err, status = Open3.capture3(RbConfig.ruby, '-I', LIB, '-e', PRELUDE + body)
    assert status.success?, "exited #{status.exitstatus}: #{err}"
    out.lines.last&.chomp
  end

  # Bug: a Ledger constant redefined before the first ledger made it an
  # instance of the new class, without any of the gem's methods.
  def test_a_redefined_ledger_constant_does_not_change_the_ledger_class
    assert_equal 'true in_progress true 2', isolated(<<~RUBY)
      original = D::Ledger
      D.send(:remove_const, :Ledger)
      D.const_set(:Ledger, Class.new)
      ledger = original.new
      ledger.claim(42, 0, 10)
      capped = ledger.with_partition_cap(2)
      puts [ledger.instance_of?(original), ledger.claim(42, 1, 10), capped.instance_of?(original),
            capped.partition_cap].join(' ')
    RUBY
  end

  # Bug: building the first ledger ran `Object.const_get`, Ruby code in the
  # middle of a native call. No Ruby runs while one is built now.
  def test_building_a_ledger_runs_no_const_get
    assert_equal '0 Ledger Ledger Ledger', isolated(<<~RUBY)
      hits = 0
      Object.define_singleton_method(:const_get) do |*args|
        hits += 1
        super(*args)
      end
      built = [D::Ledger.new, D::Ledger.with_scope(3), D::Ledger.new.with_partition_cap(2)]
      puts [hits, *built.map { it.class.name.split('::').last }].join(' ')
    RUBY
  end

  # Bug: a redefined `Object.const_get` that raised hit an `unwrap` in the
  # extension: an uncatchable fatal, and the process exited.
  def test_a_raising_const_get_does_not_kill_the_process
    assert_equal '3 2', isolated(<<~RUBY)
      Object.define_singleton_method(:const_get) { |*| raise IOError, 'lookup ran while building' }
      puts [D::Ledger.with_scope(3).scope, D::Ledger.new.with_partition_cap(2).partition_cap].join(' ')
    RUBY
  end
end
