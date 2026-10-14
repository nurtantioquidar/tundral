Imports System
Imports System.Collections.Generic

Namespace Shop.Models
    Public Class User
        Inherits Entity
        Implements IComparable(Of User)

        Public Property Name As String

        Public Sub New(name As String)
            Me.Name = name
        End Sub

        Public Function CompareTo(other As User) As Integer Implements IComparable(Of User).CompareTo
            Return String.Compare(Name, other.Name)
        End Function
    End Class
End Namespace
